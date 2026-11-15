//! Windows 11 System Media Transport Controls (SMTC) integration.
//!
//! Attaches WinRT `SystemMediaTransportControls` to the main `HWND` via
//! `ISystemMediaTransportControlsInterop::GetForWindow`, publishes playback status,
//! metadata, thumbnail, and timeline position, and forwards media key / flyout
//! button presses (`Play`, `Pause`, `Stop`, `Next`, `Previous`) to the player.

use std::path::Path;
use std::sync::mpsc;

use windows::core::{Ref, HSTRING};
use windows::Foundation::{TimeSpan, TypedEventHandler, Uri};
use windows::Media::{
    MediaPlaybackStatus, MediaPlaybackType, SystemMediaTransportControls,
    SystemMediaTransportControlsButton, SystemMediaTransportControlsButtonPressedEventArgs,
    SystemMediaTransportControlsTimelineProperties,
};
use windows::Storage::Streams::RandomAccessStreamReference;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::WinRT::{
    ISystemMediaTransportControlsInterop, RoGetActivationFactory, RoInitialize,
    RO_INIT_MULTITHREADED,
};

use crate::error::{Result, Win32ShellError};

/// Playback state mirrored to Windows SMTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtcPlaybackStatus {
    Closed,
    Stopped,
    Playing,
    Paused,
}

impl From<SmtcPlaybackStatus> for MediaPlaybackStatus {
    fn from(value: SmtcPlaybackStatus) -> Self {
        match value {
            SmtcPlaybackStatus::Closed => MediaPlaybackStatus::Closed,
            SmtcPlaybackStatus::Stopped => MediaPlaybackStatus::Stopped,
            SmtcPlaybackStatus::Playing => MediaPlaybackStatus::Playing,
            SmtcPlaybackStatus::Paused => MediaPlaybackStatus::Paused,
        }
    }
}

/// Commands triggered from Windows SMTC flyout or hardware media keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtcCommand {
    Play,
    Pause,
    Stop,
    Next,
    Previous,
    FastForward,
    Rewind,
}

/// Media metadata displayed in the Windows 11 Quick Settings media flyout.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SmtcMetadata {
    pub title: String,
    pub subtitle_or_artist: String,
    pub thumbnail_path: Option<String>,
    pub is_audio_only: bool,
}

/// Timeline properties (milliseconds) displayed in the Windows 11 media flyout.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SmtcTimeline {
    pub start_ms: i64,
    pub end_ms: i64,
    pub position_ms: i64,
    pub min_seek_ms: i64,
    pub max_seek_ms: i64,
}

fn ms_to_timespan(ms: i64) -> TimeSpan {
    TimeSpan {
        Duration: ms.saturating_mul(10_000),
    }
}

/// Windows 11 SMTC controller bound to a specific top-level `HWND`.
pub struct SmtcController {
    controls: SystemMediaTransportControls,
    button_token: i64,
}

impl SmtcController {
    /// Attach SMTC to `hwnd` and return `(controller, command_receiver)`.
    pub fn attach(hwnd: HWND) -> Result<(Self, mpsc::Receiver<SmtcCommand>)> {
        if hwnd.0.is_null() {
            return Err(Win32ShellError::InvalidHwnd);
        }

        // Initialize WinRT if not already initialized on this thread; RPC_E_CHANGED_MODE is fine.
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };

        let class_id = HSTRING::from("Windows.Media.SystemMediaTransportControls");
        let interop: ISystemMediaTransportControlsInterop =
            unsafe { RoGetActivationFactory(&class_id) }.map_err(|e| Win32ShellError::HResult {
                api: "RoGetActivationFactory(SystemMediaTransportControls)",
                hr: e.code().0,
                message: e.message(),
            })?;

        let controls: SystemMediaTransportControls = unsafe { interop.GetForWindow(hwnd) }
            .map_err(|e| Win32ShellError::HResult {
                api: "ISystemMediaTransportControlsInterop::GetForWindow",
                hr: e.code().0,
                message: e.message(),
            })?;

        let map_hr = |api: &'static str, e: windows::core::Error| Win32ShellError::HResult {
            api,
            hr: e.code().0,
            message: e.message(),
        };

        controls
            .SetIsEnabled(true)
            .map_err(|e| map_hr("SetIsEnabled", e))?;
        controls
            .SetIsPlayEnabled(true)
            .map_err(|e| map_hr("SetIsPlayEnabled", e))?;
        controls
            .SetIsPauseEnabled(true)
            .map_err(|e| map_hr("SetIsPauseEnabled", e))?;
        controls
            .SetIsStopEnabled(true)
            .map_err(|e| map_hr("SetIsStopEnabled", e))?;
        controls
            .SetIsNextEnabled(true)
            .map_err(|e| map_hr("SetIsNextEnabled", e))?;
        controls
            .SetIsPreviousEnabled(true)
            .map_err(|e| map_hr("SetIsPreviousEnabled", e))?;

        let (tx, rx) = mpsc::channel();
        let handler = TypedEventHandler::new(
            move |_sender: Ref<SystemMediaTransportControls>,
                  args: Ref<SystemMediaTransportControlsButtonPressedEventArgs>| {
                if let Some(args) = args.as_ref() {
                    if let Ok(btn) = args.Button() {
                        let cmd = match btn {
                            SystemMediaTransportControlsButton::Play => Some(SmtcCommand::Play),
                            SystemMediaTransportControlsButton::Pause => Some(SmtcCommand::Pause),
                            SystemMediaTransportControlsButton::Stop => Some(SmtcCommand::Stop),
                            SystemMediaTransportControlsButton::Next => Some(SmtcCommand::Next),
                            SystemMediaTransportControlsButton::Previous => {
                                Some(SmtcCommand::Previous)
                            }
                            SystemMediaTransportControlsButton::FastForward => {
                                Some(SmtcCommand::FastForward)
                            }
                            SystemMediaTransportControlsButton::Rewind => Some(SmtcCommand::Rewind),
                            _ => None,
                        };
                        if let Some(cmd) = cmd {
                            let _ = tx.send(cmd);
                        }
                    }
                }
                Ok(())
            },
        );

        let button_token = controls
            .ButtonPressed(&handler)
            .map_err(|e| map_hr("ButtonPressed", e))?;

        Ok((
            Self {
                controls,
                button_token,
            },
            rx,
        ))
    }

    pub fn set_playback_status(&self, status: SmtcPlaybackStatus) -> Result<()> {
        self.controls
            .SetPlaybackStatus(status.into())
            .map_err(|e| Win32ShellError::HResult {
                api: "SetPlaybackStatus",
                hr: e.code().0,
                message: e.message(),
            })
    }

    pub fn set_metadata(&self, meta: &SmtcMetadata) -> Result<()> {
        let updater = self
            .controls
            .DisplayUpdater()
            .map_err(|e| Win32ShellError::HResult {
                api: "DisplayUpdater",
                hr: e.code().0,
                message: e.message(),
            })?;

        if meta.is_audio_only {
            let _ = updater.SetType(MediaPlaybackType::Music);
            if let Ok(music) = updater.MusicProperties() {
                let _ = music.SetTitle(&HSTRING::from(meta.title.as_str()));
                let _ = music.SetArtist(&HSTRING::from(meta.subtitle_or_artist.as_str()));
            }
        } else {
            let _ = updater.SetType(MediaPlaybackType::Video);
            if let Ok(video) = updater.VideoProperties() {
                let _ = video.SetTitle(&HSTRING::from(meta.title.as_str()));
                let _ = video.SetSubtitle(&HSTRING::from(meta.subtitle_or_artist.as_str()));
            }
        }

        if let Some(thumb) = &meta.thumbnail_path {
            let p = Path::new(thumb);
            if p.exists() {
                let normalized = p.to_string_lossy().replace('\\', "/");
                let uri_str = if normalized.starts_with("file:///") {
                    normalized
                } else {
                    format!("file:///{normalized}")
                };
                if let Ok(uri) = Uri::CreateUri(&HSTRING::from(uri_str)) {
                    if let Ok(stream_ref) = RandomAccessStreamReference::CreateFromUri(&uri) {
                        let _ = updater.SetThumbnail(&stream_ref);
                    }
                }
            }
        }

        updater.Update().map_err(|e| Win32ShellError::HResult {
            api: "DisplayUpdater::Update",
            hr: e.code().0,
            message: e.message(),
        })
    }

    pub fn set_timeline(&self, timeline: SmtcTimeline) -> Result<()> {
        let props = SystemMediaTransportControlsTimelineProperties::new().map_err(|e| {
            Win32ShellError::HResult {
                api: "SystemMediaTransportControlsTimelineProperties::new",
                hr: e.code().0,
                message: e.message(),
            }
        })?;

        let _ = props.SetStartTime(ms_to_timespan(timeline.start_ms.max(0)));
        let _ = props.SetEndTime(ms_to_timespan(timeline.end_ms.max(timeline.start_ms)));
        let _ = props.SetPosition(ms_to_timespan(timeline.position_ms.max(0)));
        let _ = props.SetMinSeekTime(ms_to_timespan(timeline.min_seek_ms.max(0)));
        let _ = props.SetMaxSeekTime(ms_to_timespan(
            timeline.max_seek_ms.max(timeline.min_seek_ms),
        ));

        self.controls
            .UpdateTimelineProperties(&props)
            .map_err(|e| Win32ShellError::HResult {
                api: "UpdateTimelineProperties",
                hr: e.code().0,
                message: e.message(),
            })
    }

    pub fn set_navigation_enabled(&self, can_next: bool, can_previous: bool) -> Result<()> {
        let _ = self.controls.SetIsNextEnabled(can_next);
        let _ = self.controls.SetIsPreviousEnabled(can_previous);
        Ok(())
    }
}

impl Drop for SmtcController {
    fn drop(&mut self) {
        let _ = self.controls.RemoveButtonPressed(self.button_token);
        let _ = self.controls.SetIsEnabled(false);
    }
}
