//! AccessKit accessibility tree builder for Hybrid Player.
//! Enforces modal inertness (background controls excluded when a modal is open),
//! ARIA roles, slider ranges, toggle states, live OSD announcements, and hit bounds.

use accesskit::{Action, Live, Node, NodeId, Rect, Role, Toggled, TreeId, TreeInfo, TreeUpdate};

use crate::primitives::{HitRegion, HitTarget, RectPx, VirtualSlice};
use crate::state::{ModalId, RecordingIndicatorMode, RepeatMode, UiState, BG_CONTROLS_CATALOG};

pub const A11Y_ROOT_ID: NodeId = NodeId(1);
pub const A11Y_TITLEBAR_ID: NodeId = NodeId(2);
pub const A11Y_WELCOME_ID: NodeId = NodeId(3);
pub const A11Y_CONTROLS_ID: NodeId = NodeId(4);
pub const A11Y_PLAYLIST_ID: NodeId = NodeId(5);
pub const A11Y_MODAL_ID: NodeId = NodeId(6);
pub const A11Y_LIVE_OSD_ID: NodeId = NodeId(7);

fn to_ak_rect(r: RectPx) -> Rect {
    Rect {
        x0: r.x as f64,
        y0: r.y as f64,
        x1: (r.x + r.width) as f64,
        y1: (r.y + r.height) as f64,
    }
}

fn find_bounds(hit_regions: &[HitRegion], target: &HitTarget) -> Option<RectPx> {
    hit_regions
        .iter()
        .find(|h| &h.target == target)
        .map(|h| h.rect)
}

struct TreeBuilder<'a> {
    nodes: Vec<(NodeId, Node)>,
    next_id: u64,
    hit_regions: &'a [HitRegion],
}

impl<'a> TreeBuilder<'a> {
    fn new(hit_regions: &'a [HitRegion]) -> Self {
        Self {
            nodes: Vec::new(),
            next_id: 100,
            hit_regions,
        }
    }

    fn alloc_id(&mut self) -> NodeId {
        let id = NodeId(self.next_id);
        self.next_id += 1;
        id
    }

    fn push_button(&mut self, parent_children: &mut Vec<NodeId>, label: &str, target: &HitTarget) {
        if let Some(bounds) = find_bounds(self.hit_regions, target) {
            let id = self.alloc_id();
            let mut node = Node::new(Role::Button);
            node.set_label(label);
            node.set_bounds(to_ak_rect(bounds));
            node.add_action(Action::Click);
            node.add_action(Action::Focus);
            self.nodes.push((id, node));
            parent_children.push(id);
        }
    }

    fn push_toggle_button(
        &mut self,
        parent_children: &mut Vec<NodeId>,
        label: &str,
        pressed: bool,
        target: &HitTarget,
    ) {
        if let Some(bounds) = find_bounds(self.hit_regions, target) {
            let id = self.alloc_id();
            let mut node = Node::new(Role::Button);
            node.set_label(label);
            node.set_toggled(Toggled::from(pressed));
            node.set_bounds(to_ak_rect(bounds));
            node.add_action(Action::Click);
            node.add_action(Action::Focus);
            self.nodes.push((id, node));
            parent_children.push(id);
        }
    }

    fn push_slider(
        &mut self,
        parent_children: &mut Vec<NodeId>,
        label: &str,
        min: f64,
        max: f64,
        value: f64,
        target: &HitTarget,
    ) {
        if let Some(bounds) = find_bounds(self.hit_regions, target) {
            let id = self.alloc_id();
            let mut node = Node::new(Role::Slider);
            node.set_label(label);
            node.set_min_numeric_value(min);
            node.set_max_numeric_value(max.max(min));
            node.set_numeric_value(value.clamp(min, max.max(min)));
            node.set_bounds(to_ak_rect(bounds));
            node.add_action(Action::Focus);
            node.add_action(Action::SetValue);
            node.add_action(Action::Increment);
            node.add_action(Action::Decrement);
            self.nodes.push((id, node));
            parent_children.push(id);
        }
    }
}

/// Builds a full `accesskit::TreeUpdate` matching the current `UiState` and emitted hit regions.
/// When `state.active_modal.is_some()`, background regions (titlebar, welcome, control bar, playlist)
/// are inert and excluded from the interactive tree so assistive tech stays confined to the modal dialog.
pub fn build_accessibility_tree(
    state: &UiState,
    hit_regions: &[HitRegion],
    playlist_slice: &VirtualSlice,
) -> TreeUpdate {
    let mut b = TreeBuilder::new(hit_regions);
    let mut root_children = Vec::new();
    let mut focus = A11Y_ROOT_ID;

    if let Some(modal) = state.active_modal {
        let mut modal_children = Vec::new();
        b.push_button(&mut modal_children, "Close Modal", &HitTarget::ModalClose);

        match modal {
            ModalId::Equalizer => {
                b.push_button(
                    &mut modal_children,
                    "Equalizer Preset",
                    &HitTarget::EqPresetSelect,
                );
                b.push_button(&mut modal_children, "Reset Equalizer", &HitTarget::EqReset);
                for idx in 0..state.equalizer.bands.len() {
                    b.push_slider(
                        &mut modal_children,
                        &format!("EQ Band {}", idx + 1),
                        -12.0,
                        12.0,
                        state.equalizer.bands[idx],
                        &HitTarget::EqBandSlider(idx),
                    );
                }
            }
            ModalId::Speed => {
                for (i, label) in [
                    "0.25x", "0.5x", "0.75x", "1x", "1.25x", "1.5x", "1.75x", "2x", "3x", "4x",
                ]
                .iter()
                .enumerate()
                {
                    b.push_button(
                        &mut modal_children,
                        &format!("Speed {label}"),
                        &HitTarget::SpeedPreset(i),
                    );
                }
                b.push_slider(
                    &mut modal_children,
                    "Playback Speed",
                    0.1,
                    4.0,
                    state.speed,
                    &HitTarget::SpeedSlider,
                );
            }
            ModalId::Subtitles => {
                b.push_button(
                    &mut modal_children,
                    "Load External Subtitle",
                    &HitTarget::SubtitleLoadExternal,
                );
                b.push_button(
                    &mut modal_children,
                    "Subtitle Delay -0.5s",
                    &HitTarget::SubDelayStep(-500),
                );
                b.push_button(
                    &mut modal_children,
                    "Subtitle Delay +0.5s",
                    &HitTarget::SubDelayStep(500),
                );
                b.push_slider(
                    &mut modal_children,
                    "Subtitle Delay",
                    -600.0,
                    600.0,
                    state.subtitle_delay_secs,
                    &HitTarget::SubDelaySlider,
                );
                b.push_slider(
                    &mut modal_children,
                    "Subtitle Font Size",
                    12.0,
                    64.0,
                    state.subtitle_font_size as f64,
                    &HitTarget::SubtitleFontSizeSlider,
                );
                b.push_slider(
                    &mut modal_children,
                    "Subtitle Background Opacity",
                    0.0,
                    1.0,
                    (state.subtitle_bg_opacity_pct as f64) / 100.0,
                    &HitTarget::SubtitleBgOpacitySlider,
                );
            }
            ModalId::Audio => {
                b.push_button(
                    &mut modal_children,
                    "Subs Mode Preset",
                    &HitTarget::AudioPresetSubs,
                );
                b.push_button(
                    &mut modal_children,
                    "Dubs Mode Preset",
                    &HitTarget::AudioPresetDubs,
                );
                b.push_button(
                    &mut modal_children,
                    "Audio Delay -100ms",
                    &HitTarget::AudioDelayStep(-100),
                );
                b.push_button(
                    &mut modal_children,
                    "Audio Delay +100ms",
                    &HitTarget::AudioDelayStep(100),
                );
            }
            ModalId::Settings => {
                b.push_button(
                    &mut modal_children,
                    "Theme",
                    &HitTarget::SettingsThemeSelect,
                );
                b.push_toggle_button(
                    &mut modal_children,
                    "Brand Fonts",
                    state.font_config.brand_font_enabled,
                    &HitTarget::SettingsBrandFontsToggle,
                );
                b.push_toggle_button(
                    &mut modal_children,
                    "Auto Resume Playback",
                    state.auto_resume,
                    &HitTarget::SettingsAutoResumeToggle,
                );
                b.push_button(
                    &mut modal_children,
                    "Motion Profile",
                    &HitTarget::SettingsMotionSelect,
                );
                b.push_button(
                    &mut modal_children,
                    "Seek Step",
                    &HitTarget::SettingsSeekStepSelect,
                );
                b.push_toggle_button(
                    &mut modal_children,
                    "Double-Click to Fullscreen",
                    state.double_click_fullscreen,
                    &HitTarget::SettingsDoubleClickToggle,
                );
                b.push_toggle_button(
                    &mut modal_children,
                    "Show Unlock Button When Locked",
                    state.show_unlock_button,
                    &HitTarget::SettingsUnlockVisToggle,
                );
                b.push_button(
                    &mut modal_children,
                    "Image Format",
                    &HitTarget::SettingsScreenshotFormatSelect,
                );
                b.push_toggle_button(
                    &mut modal_children,
                    "Auto-Organize Folders",
                    state.auto_organize_screenshots,
                    &HitTarget::SettingsAutoOrganizeToggle,
                );
                b.push_button(
                    &mut modal_children,
                    "Mouse Screenshot Button",
                    &HitTarget::SettingsMouseScreenshotSelect,
                );
                if let Some(bounds) = find_bounds(hit_regions, &HitTarget::ShortcutSearch) {
                    let id = b.alloc_id();
                    let mut input = Node::new(Role::TextInput);
                    input.set_label("Search Shortcuts");
                    input.set_value(state.shortcut_search_query.as_str());
                    input.set_bounds(to_ak_rect(bounds));
                    input.add_action(Action::Focus);
                    input.add_action(Action::SetValue);
                    b.nodes.push((id, input));
                    modal_children.push(id);
                }
                for (id_str, _cat, label, combo) in state.filtered_shortcuts().into_iter().take(52)
                {
                    let target = HitTarget::ShortcutRebind(id_str.to_string());
                    b.push_button(&mut modal_children, &format!("{label} ({combo})"), &target);
                }
                b.push_button(
                    &mut modal_children,
                    "Reset All Shortcuts",
                    &HitTarget::ShortcutResetAll,
                );
                b.push_button(
                    &mut modal_children,
                    "Check yt-dlp Update",
                    &HitTarget::YtdlpCheckUpdate,
                );
                b.push_button(
                    &mut modal_children,
                    "Sign in to YouTube",
                    &HitTarget::YoutubeSignIn,
                );
                b.push_button(
                    &mut modal_children,
                    "Import Browser Cookies",
                    &HitTarget::YoutubeImportBrowserCookies,
                );
                b.push_button(
                    &mut modal_children,
                    "Import cookies.txt",
                    &HitTarget::YoutubeImportCookiesFile,
                );
                b.push_button(
                    &mut modal_children,
                    "Open Multiple Files",
                    &HitTarget::SettingsOpenMultiple,
                );
                b.push_button(
                    &mut modal_children,
                    "Open Folder",
                    &HitTarget::SettingsOpenFolder,
                );
                b.push_button(&mut modal_children, "Quit Player", &HitTarget::SettingsQuit);
            }
            ModalId::NetworkStream => {
                if let Some(bounds) = find_bounds(hit_regions, &HitTarget::NetworkStreamInput) {
                    let id = b.alloc_id();
                    let mut input = Node::new(Role::TextInput);
                    input.set_label("Network Stream URL");
                    input.set_value(state.network_stream_url.as_str());
                    input.set_bounds(to_ak_rect(bounds));
                    input.add_action(Action::Focus);
                    input.add_action(Action::SetValue);
                    b.nodes.push((id, input));
                    modal_children.push(id);
                }
                b.push_button(
                    &mut modal_children,
                    "Play Stream",
                    &HitTarget::NetworkStreamOpen,
                );
            }
            ModalId::BgSettings => {
                b.push_button(
                    &mut modal_children,
                    "Choose Welcome Effect",
                    &HitTarget::BgEffectPickerToggle,
                );
                b.push_button(
                    &mut modal_children,
                    "Reset Current Effect",
                    &HitTarget::BgResetCurrent,
                );
                for ctrl in BG_CONTROLS_CATALOG
                    .iter()
                    .filter(|c| c.bg == state.welcome_background)
                {
                    b.push_button(
                        &mut modal_children,
                        ctrl.label,
                        &HitTarget::BgControlInput(ctrl.id.to_string()),
                    );
                }
                if state.bg_accordion_open {
                    for bg in [
                        hybrid_shaders::WelcomeBackgroundKind::None,
                        hybrid_shaders::WelcomeBackgroundKind::Dither,
                        hybrid_shaders::WelcomeBackgroundKind::Particles,
                        hybrid_shaders::WelcomeBackgroundKind::Faulty,
                        hybrid_shaders::WelcomeBackgroundKind::Gridmotion,
                    ] {
                        b.push_button(
                            &mut modal_children,
                            &format!("Effect {bg:?}"),
                            &HitTarget::BgAccordionCard(bg),
                        );
                    }
                }
            }
        }

        let mut modal_node = Node::new(Role::Dialog);
        modal_node.set_label(match modal {
            ModalId::Equalizer => "Audio Equalizer",
            ModalId::Speed => "Playback Speed",
            ModalId::Subtitles => "Subtitles",
            ModalId::Audio => "Audio Tracks & Sync",
            ModalId::Settings => "Settings",
            ModalId::NetworkStream => "Open Network Stream",
            ModalId::BgSettings => "Welcome Effects",
        });
        modal_node.set_modal();
        modal_node.set_children(modal_children);
        b.nodes.push((A11Y_MODAL_ID, modal_node));
        root_children.push(A11Y_MODAL_ID);
        focus = A11Y_MODAL_ID;
    } else {
        // Titlebar landmark
        let mut titlebar_children = Vec::new();
        b.push_button(
            &mut titlebar_children,
            "Minimize",
            &HitTarget::TitlebarMinimize,
        );
        b.push_button(
            &mut titlebar_children,
            "Maximize",
            &HitTarget::TitlebarMaximize,
        );
        b.push_button(&mut titlebar_children, "Close", &HitTarget::TitlebarClose);
        let mut tb = Node::new(Role::Region);
        tb.set_label("Titlebar");
        tb.set_children(titlebar_children);
        b.nodes.push((A11Y_TITLEBAR_ID, tb));
        root_children.push(A11Y_TITLEBAR_ID);

        // Welcome screen landmark
        if state.welcome_active {
            let mut welcome_children = Vec::new();
            b.push_button(&mut welcome_children, "Open File", &HitTarget::OpenFile);
            b.push_button(&mut welcome_children, "Open Link", &HitTarget::OpenLink);
            for idx in 0..state.recent_files.len().min(5) {
                let title = &state.recent_files[idx].name;
                b.push_button(
                    &mut welcome_children,
                    &format!("Recent: {title}"),
                    &HitTarget::RecentFile(idx),
                );
            }
            let mut w = Node::new(Role::Region);
            w.set_label("Welcome Screen");
            w.set_children(welcome_children);
            b.nodes.push((A11Y_WELCOME_ID, w));
            root_children.push(A11Y_WELCOME_ID);
        }

        // Playback controls landmark (hidden #btnScreenshot intentionally omitted!)
        {
            let mut ctrl_children = Vec::new();
            b.push_slider(
                &mut ctrl_children,
                "Seek Progress",
                0.0,
                state.duration_secs.max(1.0),
                state.current_secs,
                &HitTarget::ProgressSeek,
            );
            b.push_button(&mut ctrl_children, "Previous Track", &HitTarget::PrevTrack);
            b.push_toggle_button(
                &mut ctrl_children,
                if state.playing { "Pause" } else { "Play" },
                state.playing,
                &HitTarget::PlayPause,
            );
            b.push_button(&mut ctrl_children, "Next Track", &HitTarget::NextTrack);
            b.push_toggle_button(
                &mut ctrl_children,
                "Mute",
                state.muted,
                &HitTarget::MuteToggle,
            );
            b.push_slider(
                &mut ctrl_children,
                "Volume",
                0.0,
                if state.volume_boost_enabled {
                    200.0
                } else {
                    100.0
                },
                state.volume_pct as f64,
                &HitTarget::VolumeSlider,
            );
            b.push_button(
                &mut ctrl_children,
                "Welcome Effects",
                &HitTarget::BgSettingsToggle,
            );
            b.push_button(
                &mut ctrl_children,
                "Playback Speed",
                &HitTarget::SpeedButton,
            );
            b.push_button(&mut ctrl_children, "Equalizer", &HitTarget::EqualizerButton);
            b.push_button(
                &mut ctrl_children,
                "Audio Track",
                &HitTarget::AudioTrackButton,
            );
            b.push_button(&mut ctrl_children, "Subtitles", &HitTarget::SubtitlesButton);
            b.push_toggle_button(
                &mut ctrl_children,
                "Playlist",
                state.playlist.open,
                &HitTarget::PlaylistToggle,
            );
            b.push_toggle_button(
                &mut ctrl_children,
                "Repeat",
                state.repeat != RepeatMode::None,
                &HitTarget::RepeatToggle,
            );
            b.push_toggle_button(
                &mut ctrl_children,
                "Shuffle",
                state.shuffle,
                &HitTarget::ShuffleToggle,
            );
            b.push_button(&mut ctrl_children, "A-B Loop", &HitTarget::AbLoopToggle);
            b.push_toggle_button(
                &mut ctrl_children,
                "Lock Controls",
                state.locked,
                &HitTarget::LockToggle,
            );
            b.push_button(
                &mut ctrl_children,
                "Video Quality",
                &HitTarget::YoutubeQualityButton,
            );
            b.push_button(&mut ctrl_children, "Settings", &HitTarget::SettingsButton);
            b.push_button(
                &mut ctrl_children,
                "Fullscreen",
                &HitTarget::FullscreenToggle,
            );

            let mut c = Node::new(Role::Region);
            c.set_label("Playback Controls");
            c.set_children(ctrl_children);
            b.nodes.push((A11Y_CONTROLS_ID, c));
            root_children.push(A11Y_CONTROLS_ID);
        }

        // Playlist sidebar landmark
        if state.playlist.open {
            let mut pl_children = Vec::new();
            b.push_button(
                &mut pl_children,
                "Close Playlist",
                &HitTarget::PlaylistClose,
            );
            b.push_button(&mut pl_children, "Add Files", &HitTarget::PlaylistAddFiles);
            b.push_toggle_button(
                &mut pl_children,
                "Shuffle Playlist",
                state.shuffle,
                &HitTarget::PlaylistShuffle,
            );
            b.push_toggle_button(
                &mut pl_children,
                "Repeat Playlist",
                state.repeat != RepeatMode::None,
                &HitTarget::PlaylistRepeat,
            );
            b.push_button(
                &mut pl_children,
                "Clear Playlist",
                &HitTarget::PlaylistClear,
            );

            if let Some(bounds) = find_bounds(hit_regions, &HitTarget::PlaylistSearch) {
                let id = b.alloc_id();
                let mut search = Node::new(Role::TextInput);
                search.set_label("Search Playlist");
                search.set_value(state.playlist.search_query.as_str());
                search.set_bounds(to_ak_rect(bounds));
                search.add_action(Action::Focus);
                search.add_action(Action::SetValue);
                b.nodes.push((id, search));
                pl_children.push(id);
            }

            let filtered = state.playlist.filtered_items();
            let mut list_children = Vec::new();
            for row_pos in playlist_slice.start_index..playlist_slice.end_index {
                if let Some((orig_idx, item)) = filtered.get(row_pos) {
                    if let Some(bounds) =
                        find_bounds(hit_regions, &HitTarget::PlaylistRow(*orig_idx))
                    {
                        let row_id = b.alloc_id();
                        let mut row = Node::new(Role::ListItem);
                        row.set_label(item.name.as_str());
                        row.set_bounds(to_ak_rect(bounds));
                        row.add_action(Action::Click);
                        b.nodes.push((row_id, row));
                        list_children.push(row_id);
                    }
                }
            }
            let list_id = b.alloc_id();
            let mut list = Node::new(Role::List);
            list.set_label("Playlist Items");
            list.set_children(list_children);
            b.nodes.push((list_id, list));
            pl_children.push(list_id);

            let mut pl = Node::new(Role::Region);
            pl.set_label("Playlist Drawer");
            pl.set_children(pl_children);
            b.nodes.push((A11Y_PLAYLIST_ID, pl));
            root_children.push(A11Y_PLAYLIST_ID);
        }
    }

    // Live OSD announcements (Volume OSD, Recording Indicator, Toasts)
    let mut live_messages = Vec::new();
    if state.volume_osd.visible {
        live_messages.push(format!("Volume {}%", state.volume_osd.percent));
    }
    match state.recording_indicator {
        RecordingIndicatorMode::BurstActive { count } => {
            live_messages.push(format!("Burst capture {count} frames"));
        }
        RecordingIndicatorMode::ClipActive { elapsed_ds } => {
            live_messages.push(format!("Recording clip {:.1}s", elapsed_ds as f32 / 10.0));
        }
        RecordingIndicatorMode::ClipSaving => {
            live_messages.push("Saving clip".to_string());
        }
        RecordingIndicatorMode::Saved { label_frames } => {
            live_messages.push(if label_frames {
                "Burst saved".to_string()
            } else {
                "Clip saved".to_string()
            });
        }
        RecordingIndicatorMode::Hidden => {}
    }
    if let Some(last_toast) = state.toasts.last() {
        live_messages.push(last_toast.message.clone());
    }
    if !live_messages.is_empty() {
        let mut status = Node::new(Role::Status);
        status.set_live(Live::Polite);
        status.set_label(live_messages.join("; "));
        b.nodes.push((A11Y_LIVE_OSD_ID, status));
        root_children.push(A11Y_LIVE_OSD_ID);
    }

    let mut root = Node::new(Role::Window);
    root.set_label(state.window_title.as_str());
    root.set_bounds(to_ak_rect(RectPx::new(
        0.0,
        0.0,
        state.viewport_w.max(1.0),
        state.viewport_h.max(1.0),
    )));
    root.set_children(root_children);
    b.nodes.insert(0, (A11Y_ROOT_ID, root));

    TreeUpdate {
        nodes: b.nodes,
        tree: Some(TreeInfo {
            root: A11Y_ROOT_ID,
            toolkit_name: Some("hybrid-ui".to_string()),
            toolkit_version: Some(env!("CARGO_PKG_VERSION").to_string()),
        }),
        tree_id: TreeId::ROOT,
        focus,
    }
}
