//! Auto-extracted 1:1 SVG catalog covering all 64 main UI SVGs + 3 companion auth SVGs.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SvgId {
    /// Titlebar dotted-triangle brand-mark.svg geometry (not brand-lockup.svg)
    BrandMarkTitlebar,
    /// #btnMinimize
    TitlebarMinimize,
    /// #btnMaximize
    TitlebarMaximize,
    /// #btnClose
    TitlebarClose,
    /// #btnOpenFile
    WelcomeOpenFile,
    /// #btnOpenLink
    WelcomeOpenLink,
    /// #recordingIconCam
    RecordingCam,
    /// #recordingStopSquare
    RecordingStopSquare,
    /// #recordingStopFolder
    RecordingStopFolder,
    /// #recordingIconFolder
    RecordingSavedFolder,
    /// #unlockOverlay
    UnlockPadlock,
    /// #btnPrev
    CtrlPrev,
    /// #btnPlay .icon-play
    CtrlPlay,
    /// #btnPlay .icon-pause
    CtrlPause,
    /// #btnNext
    CtrlNext,
    /// #btnVolume .icon-vol-high
    CtrlVolumeHigh,
    /// #btnVolume .icon-vol-low
    CtrlVolumeLow,
    /// #btnVolume .icon-vol-mute
    CtrlVolumeMute,
    /// #bgSettingsToggle
    CtrlBgSparkles,
    /// #btnEqualizer
    CtrlEqualizer,
    /// #btnAudioTrack
    CtrlAudioTrack,
    /// #btnSubtitles
    CtrlSubtitles,
    /// #btnPlaylist
    CtrlPlaylist,
    /// #btnRepeat .icon-repeat
    CtrlRepeat,
    /// #btnShuffle
    CtrlShuffle,
    /// #btnABLoop .icon-ab-loop
    CtrlAbLoop,
    /// #btnLock
    CtrlLock,
    /// #btnSettings
    CtrlSettings,
    /// #btnFullscreen .icon-fullscreen-enter
    CtrlFullscreenEnter,
    /// #btnFullscreen .icon-fullscreen-exit
    CtrlFullscreenExit,
    /// #btnClosePlaylist
    PlaylistClose,
    /// #btnAddToPlaylist
    PlaylistAddFiles,
    /// #btnShufflePlaylist
    PlaylistShuffle,
    /// #btnRepeatPlaylist .icon-repeat
    PlaylistRepeat,
    /// #btnClearPlaylist
    PlaylistClear,
    /// #equalizerModal .modal-close
    ModalCloseEqualizer,
    /// #speedModal .modal-close
    ModalCloseSpeed,
    /// #subtitleModal .modal-close
    ModalCloseSubtitle,
    /// #settingsModal .modal-close
    ModalCloseSettings,
    /// #ytCookiesCard .yt-real-logo
    SettingsYoutubeLogo,
    /// #networkStreamModal .modal-close
    ModalCloseNetworkStream,
    /// .bg-accordion-item[data-bg=dither] check
    AccordionCheckDither,
    /// .bg-accordion-item[data-bg=particles] check
    AccordionCheckParticles,
    /// .bg-accordion-item[data-bg=faulty] check
    AccordionCheckFaulty,
    /// .bg-accordion-item[data-bg=gridmotion] check
    AccordionCheckGridmotion,
    /// .bg-accordion-item[data-bg=none] check
    AccordionCheckNone,
    /// #bgSettingsModal .modal-close
    ModalCloseBgSettings,
    /// #bgEffectPickerBtn chevron
    BgPickerChevron,
    /// toast.js:90 error pill close
    ToastErrorClose,
    /// toast.js:126 update pill download icon
    ToastUpdateBadge,
    /// toast.js:144 Update Now button icon
    ToastUpdateAction,
    /// toast.js:150 update pill dismiss
    ToastUpdateClose,
    /// toast.js:195 update success check
    ToastSuccessCheck,
    /// app.js:416 exit-fullscreen monitor
    TitlebarExitFullscreenMonitor,
    /// app.js:426 restore stacked squares
    TitlebarRestoreStacked,
    /// app.js:435 maximize square
    TitlebarMaximizeSquare,
    /// app.js:882 recent-item play triangle
    WelcomeRecentPlay,
    /// controls.js:1579 skip forward double-arrow
    SkipForwardArrows,
    /// controls.js:1586 skip backward double-arrow
    SkipBackwardArrows,
    /// player.js:1784 carousel prev chevron
    CarouselPrevArrow,
    /// player.js:1798 carousel next chevron
    CarouselNextArrow,
    /// settings.js:1529 custom select checkmark
    CustomSelectCheck,
    /// settings.js:2175 per-shortcut reset arrow
    ShortcutRowReset,
    /// playlist.js:543 empty playlist icon
    PlaylistEmptyList,
    /// youtube-auth.js:888 YouTube header logo
    AuthYoutubeLogo,
    /// youtube-auth.js:899 verified security shield
    AuthSecurityShield,
    /// youtube-auth.js:908 step 1 completed check
    AuthStepDoneCheck,
}

impl SvgId {
    pub const MAIN_UI_COUNT: usize = 64;
    pub const COMPANION_AUTH_COUNT: usize = 3;
    pub const ALL: [SvgId; 67] = [
        Self::BrandMarkTitlebar,
        Self::TitlebarMinimize,
        Self::TitlebarMaximize,
        Self::TitlebarClose,
        Self::WelcomeOpenFile,
        Self::WelcomeOpenLink,
        Self::RecordingCam,
        Self::RecordingStopSquare,
        Self::RecordingStopFolder,
        Self::RecordingSavedFolder,
        Self::UnlockPadlock,
        Self::CtrlPrev,
        Self::CtrlPlay,
        Self::CtrlPause,
        Self::CtrlNext,
        Self::CtrlVolumeHigh,
        Self::CtrlVolumeLow,
        Self::CtrlVolumeMute,
        Self::CtrlBgSparkles,
        Self::CtrlEqualizer,
        Self::CtrlAudioTrack,
        Self::CtrlSubtitles,
        Self::CtrlPlaylist,
        Self::CtrlRepeat,
        Self::CtrlShuffle,
        Self::CtrlAbLoop,
        Self::CtrlLock,
        Self::CtrlSettings,
        Self::CtrlFullscreenEnter,
        Self::CtrlFullscreenExit,
        Self::PlaylistClose,
        Self::PlaylistAddFiles,
        Self::PlaylistShuffle,
        Self::PlaylistRepeat,
        Self::PlaylistClear,
        Self::ModalCloseEqualizer,
        Self::ModalCloseSpeed,
        Self::ModalCloseSubtitle,
        Self::ModalCloseSettings,
        Self::SettingsYoutubeLogo,
        Self::ModalCloseNetworkStream,
        Self::AccordionCheckDither,
        Self::AccordionCheckParticles,
        Self::AccordionCheckFaulty,
        Self::AccordionCheckGridmotion,
        Self::AccordionCheckNone,
        Self::ModalCloseBgSettings,
        Self::BgPickerChevron,
        Self::ToastErrorClose,
        Self::ToastUpdateBadge,
        Self::ToastUpdateAction,
        Self::ToastUpdateClose,
        Self::ToastSuccessCheck,
        Self::TitlebarExitFullscreenMonitor,
        Self::TitlebarRestoreStacked,
        Self::TitlebarMaximizeSquare,
        Self::WelcomeRecentPlay,
        Self::SkipForwardArrows,
        Self::SkipBackwardArrows,
        Self::CarouselPrevArrow,
        Self::CarouselNextArrow,
        Self::CustomSelectCheck,
        Self::ShortcutRowReset,
        Self::PlaylistEmptyList,
        Self::AuthYoutubeLogo,
        Self::AuthSecurityShield,
        Self::AuthStepDoneCheck,
    ];

    pub const fn is_companion_auth(self) -> bool {
        matches!(
            self,
            Self::AuthYoutubeLogo | Self::AuthSecurityShield | Self::AuthStepDoneCheck
        )
    }

    pub const fn raw_svg(self) -> &'static str {
        match self {
            Self::BrandMarkTitlebar => {
                r###"<svg width="20" height="20" viewBox="-52 0 508 400" fill="none" aria-hidden="true"> <circle cx="56" cy="218" r="8.5" fill="currentColor" /> <circle cx="56" cy="236" r="8.5" fill="currentColor" /> <circle cx="56" cy="254" r="8.5" fill="currentColor" /> <circle cx="56" cy="272" r="8.5" fill="currentColor" /> <circle cx="74" cy="164" r="8.5" fill="currentColor" /> <circle cx="74" cy="182" r="8.5" fill="currentColor" /> <circle cx="74" cy="200" r="8.5" fill="currentColor" /> <circle cx="74" cy="218" r="8.5" fill="currentColor" /> <circle cx="74" cy="236" r="8.5" fill="currentColor" /> <circle cx="74" cy="254" r="8.5" fill="currentColor" /> <circle cx="74" cy="272" r="8.5" fill="currentColor" /> <circle cx="74" cy="290" r="8.5" fill="currentColor" /> <circle cx="74" cy="308" r="8.5" fill="currentColor" /> <circle cx="92" cy="56" r="8.5" fill="currentColor" /> <circle cx="92" cy="74" r="8.5" fill="currentColor" /> <circle cx="92" cy="92" r="8.5" fill="currentColor" /> <circle cx="92" cy="110" r="8.5" fill="currentColor" /> <circle cx="92" cy="128" r="8.5" fill="currentColor" /> <circle cx="92" cy="146" r="8.5" fill="currentColor" /> <circle cx="92" cy="164" r="8.5" fill="currentColor" /> <circle cx="92" cy="182" r="8.5" fill="currentColor" /> <circle cx="92" cy="200" r="8.5" fill="currentColor" /> <circle cx="92" cy="218" r="8.5" fill="currentColor" /> <circle cx="92" cy="236" r="8.5" fill="currentColor" /> <circle cx="92" cy="254" r="8.5" fill="currentColor" /> <circle cx="92" cy="272" r="8.5" fill="currentColor" /> <circle cx="92" cy="290" r="8.5" fill="currentColor" /> <circle cx="92" cy="308" r="8.5" fill="currentColor" /> <circle cx="92" cy="326" r="8.5" fill="currentColor" /> <circle cx="110" cy="74" r="8.5" fill="currentColor" /> <circle cx="110" cy="92" r="8.5" fill="currentColor" /> <circle cx="110" cy="164" r="8.5" fill="currentColor" /> <circle cx="110" cy="182" r="8.5" fill="currentColor" /> <circle cx="110" cy="200" r="8.5" fill="currentColor" /> <circle cx="110" cy="218" r="8.5" fill="currentColor" /> <circle cx="110" cy="236" r="8.5" fill="currentColor" /> <circle cx="110" cy="254" r="8.5" fill="currentColor" /> <circle cx="110" cy="272" r="8.5" fill="currentColor" /> <circle cx="110" cy="290" r="8.5" fill="currentColor" /> <circle cx="110" cy="308" r="8.5" fill="currentColor" /> <circle cx="110" cy="326" r="8.5" fill="currentColor" /> <circle cx="110" cy="344" r="8.5" fill="currentColor" /> <circle cx="128" cy="92" r="8.5" fill="currentColor" /> <circle cx="128" cy="110" r="8.5" fill="currentColor" /> <circle cx="128" cy="164" r="8.5" fill="currentColor" /> <circle cx="128" cy="182" r="8.5" fill="currentColor" /> <circle cx="128" cy="200" r="8.5" fill="currentColor" /> <circle cx="128" cy="218" r="8.5" fill="currentColor" /> <circle cx="128" cy="236" r="8.5" fill="currentColor" /> <circle cx="128" cy="254" r="8.5" fill="currentColor" /> <circle cx="128" cy="272" r="8.5" fill="currentColor" /> <circle cx="128" cy="290" r="8.5" fill="currentColor" /> <circle cx="128" cy="308" r="8.5" fill="currentColor" /> <circle cx="128" cy="326" r="8.5" fill="currentColor" /> <circle cx="128" cy="344" r="8.5" fill="currentColor" /> <circle cx="146" cy="128" r="8.5" fill="currentColor" /> <circle cx="146" cy="164" r="8.5" fill="currentColor" /> <circle cx="146" cy="182" r="8.5" fill="currentColor" /> <circle cx="146" cy="200" r="8.5" fill="currentColor" /> <circle cx="146" cy="218" r="8.5" fill="currentColor" /> <circle cx="146" cy="236" r="8.5" fill="currentColor" /> <circle cx="146" cy="254" r="8.5" fill="currentColor" /> <circle cx="146" cy="272" r="8.5" fill="currentColor" /> <circle cx="146" cy="290" r="8.5" fill="currentColor" /> <circle cx="146" cy="308" r="8.5" fill="currentColor" /> <circle cx="146" cy="326" r="8.5" fill="currentColor" /> <circle cx="146" cy="344" r="8.5" fill="currentColor" /> <circle cx="164" cy="146" r="8.5" fill="currentColor" /> <circle cx="164" cy="164" r="8.5" fill="currentColor" /> <circle cx="164" cy="182" r="8.5" fill="currentColor" /> <circle cx="164" cy="200" r="8.5" fill="currentColor" /> <circle cx="164" cy="218" r="8.5" fill="currentColor" /> <circle cx="164" cy="236" r="8.5" fill="currentColor" /> <circle cx="164" cy="254" r="8.5" fill="currentColor" /> <circle cx="164" cy="272" r="8.5" fill="currentColor" /> <circle cx="164" cy="290" r="8.5" fill="currentColor" /> <circle cx="164" cy="308" r="8.5" fill="currentColor" /> <circle cx="164" cy="326" r="8.5" fill="currentColor" /> <circle cx="164" cy="344" r="8.5" fill="currentColor" /> <circle cx="182" cy="128" r="8.5" fill="currentColor" /> <circle cx="182" cy="146" r="8.5" fill="currentColor" /> <circle cx="182" cy="164" r="8.5" fill="currentColor" /> <circle cx="182" cy="182" r="8.5" fill="currentColor" /> <circle cx="182" cy="200" r="8.5" fill="currentColor" /> <circle cx="182" cy="308" r="8.5" fill="currentColor" /> <circle cx="182" cy="326" r="8.5" fill="currentColor" /> <circle cx="182" cy="344" r="8.5" fill="currentColor" /> <circle cx="200" cy="128" r="8.5" fill="currentColor" /> <circle cx="200" cy="146" r="8.5" fill="currentColor" /> <circle cx="200" cy="164" r="8.5" fill="currentColor" /> <circle cx="200" cy="182" r="8.5" fill="currentColor" /> <circle cx="200" cy="200" r="8.5" fill="currentColor" /> <circle cx="200" cy="218" r="8.5" fill="currentColor" /> <circle cx="200" cy="290" r="8.5" fill="currentColor" /> <circle cx="200" cy="308" r="8.5" fill="currentColor" /> <circle cx="200" cy="326" r="8.5" fill="currentColor" /> <circle cx="200" cy="344" r="8.5" fill="currentColor" /> <circle cx="218" cy="128" r="8.5" fill="currentColor" /> <circle cx="218" cy="146" r="8.5" fill="currentColor" /> <circle cx="218" cy="164" r="8.5" fill="currentColor" /> <circle cx="218" cy="182" r="8.5" fill="currentColor" /> <circle cx="218" cy="200" r="8.5" fill="currentColor" /> <circle cx="218" cy="218" r="8.5" fill="currentColor" /> <circle cx="218" cy="290" r="8.5" fill="currentColor" /> <circle cx="218" cy="308" r="8.5" fill="currentColor" /> <circle cx="218" cy="326" r="8.5" fill="currentColor" /> <circle cx="218" cy="344" r="8.5" fill="currentColor" /> <circle cx="236" cy="146" r="8.5" fill="currentColor" /> <circle cx="236" cy="164" r="8.5" fill="currentColor" /> <circle cx="236" cy="182" r="8.5" fill="currentColor" /> <circle cx="236" cy="200" r="8.5" fill="currentColor" /> <circle cx="236" cy="218" r="8.5" fill="currentColor" /> <circle cx="236" cy="236" r="8.5" fill="currentColor" /> <circle cx="236" cy="272" r="8.5" fill="currentColor" /> <circle cx="236" cy="290" r="8.5" fill="currentColor" /> <circle cx="236" cy="308" r="8.5" fill="currentColor" /> <circle cx="236" cy="326" r="8.5" fill="currentColor" /> <circle cx="236" cy="344" r="8.5" fill="currentColor" /> <circle cx="254" cy="128" r="8.5" fill="currentColor" /> <circle cx="254" cy="164" r="8.5" fill="currentColor" /> <circle cx="254" cy="182" r="8.5" fill="currentColor" /> <circle cx="254" cy="200" r="8.5" fill="currentColor" /> <circle cx="254" cy="218" r="8.5" fill="currentColor" /> <circle cx="254" cy="236" r="8.5" fill="currentColor" /> <circle cx="254" cy="254" r="8.5" fill="currentColor" /> <circle cx="254" cy="272" r="8.5" fill="currentColor" /> <circle cx="254" cy="290" r="8.5" fill="currentColor" /> <circle cx="254" cy="308" r="8.5" fill="currentColor" /> <circle cx="254" cy="326" r="8.5" fill="currentColor" /> <circle cx="254" cy="344" r="8.5" fill="currentColor" /> <circle cx="272" cy="92" r="8.5" fill="currentColor" /> <circle cx="272" cy="110" r="8.5" fill="currentColor" /> <circle cx="272" cy="164" r="8.5" fill="currentColor" /> <circle cx="272" cy="182" r="8.5" fill="currentColor" /> <circle cx="272" cy="200" r="8.5" fill="currentColor" /> <circle cx="272" cy="218" r="8.5" fill="currentColor" /> <circle cx="272" cy="236" r="8.5" fill="currentColor" /> <circle cx="272" cy="254" r="8.5" fill="currentColor" /> <circle cx="272" cy="272" r="8.5" fill="currentColor" /> <circle cx="272" cy="290" r="8.5" fill="currentColor" /> <circle cx="272" cy="308" r="8.5" fill="currentColor" /> <circle cx="272" cy="326" r="8.5" fill="currentColor" /> <circle cx="272" cy="344" r="8.5" fill="currentColor" /> <circle cx="290" cy="74" r="8.5" fill="currentColor" /> <circle cx="290" cy="92" r="8.5" fill="currentColor" /> <circle cx="290" cy="164" r="8.5" fill="currentColor" /> <circle cx="290" cy="182" r="8.5" fill="currentColor" /> <circle cx="290" cy="200" r="8.5" fill="currentColor" /> <circle cx="290" cy="218" r="8.5" fill="currentColor" /> <circle cx="290" cy="236" r="8.5" fill="currentColor" /> <circle cx="290" cy="254" r="8.5" fill="currentColor" /> <circle cx="290" cy="272" r="8.5" fill="currentColor" /> <circle cx="290" cy="290" r="8.5" fill="currentColor" /> <circle cx="290" cy="308" r="8.5" fill="currentColor" /> <circle cx="290" cy="326" r="8.5" fill="currentColor" /> <circle cx="290" cy="344" r="8.5" fill="currentColor" /> <circle cx="308" cy="56" r="8.5" fill="currentColor" /> <circle cx="308" cy="74" r="8.5" fill="currentColor" /> <circle cx="308" cy="92" r="8.5" fill="currentColor" /> <circle cx="308" cy="110" r="8.5" fill="currentColor" /> <circle cx="308" cy="128" r="8.5" fill="currentColor" /> <circle cx="308" cy="146" r="8.5" fill="currentColor" /> <circle cx="308" cy="164" r="8.5" fill="currentColor" /> <circle cx="308" cy="182" r="8.5" fill="currentColor" /> <circle cx="308" cy="200" r="8.5" fill="currentColor" /> <circle cx="308" cy="218" r="8.5" fill="currentColor" /> <circle cx="308" cy="236" r="8.5" fill="currentColor" /> <circle cx="308" cy="254" r="8.5" fill="currentColor" /> <circle cx="308" cy="272" r="8.5" fill="currentColor" /> <circle cx="308" cy="290" r="8.5" fill="currentColor" /> <circle cx="308" cy="308" r="8.5" fill="currentColor" /> <circle cx="308" cy="326" r="8.5" fill="currentColor" /> <circle cx="326" cy="164" r="8.5" fill="currentColor" /> <circle cx="326" cy="182" r="8.5" fill="currentColor" /> <circle cx="326" cy="200" r="8.5" fill="currentColor" /> <circle cx="326" cy="218" r="8.5" fill="currentColor" /> <circle cx="326" cy="236" r="8.5" fill="currentColor" /> <circle cx="326" cy="254" r="8.5" fill="currentColor" /> <circle cx="326" cy="272" r="8.5" fill="currentColor" /> <circle cx="326" cy="290" r="8.5" fill="currentColor" /> <circle cx="326" cy="308" r="8.5" fill="currentColor" /> <circle cx="344" cy="218" r="8.5" fill="currentColor" /> <circle cx="344" cy="236" r="8.5" fill="currentColor" /> <circle cx="344" cy="254" r="8.5" fill="currentColor" /> <circle cx="344" cy="272" r="8.5" fill="currentColor" /> <circle cx="45.38" cy="222.31" r="4.35" fill="currentColor" /> <circle cx="28.31" cy="219.85" r="4.03" fill="currentColor" /> <circle cx="11.23" cy="217.38" r="3.70" fill="currentColor" /> <circle cx="-5.85" cy="214.92" r="3.37" fill="currentColor" /> <circle cx="-22.92" cy="212.46" r="3.05" fill="currentColor" /> <circle cx="-40.00" cy="210.00" r="2.72" fill="currentColor" /> <circle cx="41.69" cy="258.00" r="4.35" fill="currentColor" /> <circle cx="24.15" cy="258.00" r="4.03" fill="currentColor" /> <circle cx="6.62" cy="258.00" r="3.70" fill="currentColor" /> <circle cx="-10.92" cy="258.00" r="3.37" fill="currentColor" /> <circle cx="-28.46" cy="258.00" r="3.05" fill="currentColor" /> <circle cx="-46.00" cy="258.00" r="2.72" fill="currentColor" /> <circle cx="45.38" cy="293.69" r="4.35" fill="currentColor" /> <circle cx="28.31" cy="296.15" r="4.03" fill="currentColor" /> <circle cx="11.23" cy="298.62" r="3.70" fill="currentColor" /> <circle cx="-5.85" cy="301.08" r="3.37" fill="currentColor" /> <circle cx="-22.92" cy="303.54" r="3.05" fill="currentColor" /> <circle cx="-40.00" cy="306.00" r="2.72" fill="currentColor" /> <circle cx="354.62" cy="222.31" r="4.35" fill="currentColor" /> <circle cx="371.69" cy="219.85" r="4.03" fill="currentColor" /> <circle cx="388.77" cy="217.38" r="3.70" fill="currentColor" /> <circle cx="405.85" cy="214.92" r="3.37" fill="currentColor" /> <circle cx="422.92" cy="212.46" r="3.05" fill="currentColor" /> <circle cx="440.00" cy="210.00" r="2.72" fill="currentColor" /> <circle cx="358.31" cy="258.00" r="4.35" fill="currentColor" /> <circle cx="375.85" cy="258.00" r="4.03" fill="currentColor" /> <circle cx="393.38" cy="258.00" r="3.70" fill="currentColor" /> <circle cx="410.92" cy="258.00" r="3.37" fill="currentColor" /> <circle cx="428.46" cy="258.00" r="3.05" fill="currentColor" /> <circle cx="446.00" cy="258.00" r="2.72" fill="currentColor" /> <circle cx="354.62" cy="293.69" r="4.35" fill="currentColor" /> <circle cx="371.69" cy="296.15" r="4.03" fill="currentColor" /> <circle cx="388.77" cy="298.62" r="3.70" fill="currentColor" /> <circle cx="405.85" cy="301.08" r="3.37" fill="currentColor" /> <circle cx="422.92" cy="303.54" r="3.05" fill="currentColor" /> <circle cx="440.00" cy="306.00" r="2.72" fill="currentColor" /> </svg>"###
            }
            Self::TitlebarMinimize => {
                r###"<svg width="12" height="12" viewBox="0 0 12 12"> <rect y="5" width="12" height="1.5" fill="currentColor" /> </svg>"###
            }
            Self::TitlebarMaximize => {
                r###"<svg width="12" height="12" viewBox="0 0 12 12"> <rect x="1" y="1" width="10" height="10" rx="1.5" fill="none" stroke="currentColor" stroke-width="1.3" /> </svg>"###
            }
            Self::TitlebarClose => {
                r###"<svg width="12" height="12" viewBox="0 0 12 12"> <line x1="1" y1="1" x2="11" y2="11" stroke="currentColor" stroke-width="1.5" /> <line x1="11" y1="1" x2="1" y2="11" stroke="currentColor" stroke-width="1.5" /> </svg>"###
            }
            Self::WelcomeOpenFile => {
                r###"<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"> <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" /> </svg>"###
            }
            Self::WelcomeOpenLink => {
                r###"<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"> <path d="M10 14a5 5 0 0 0 7.07 0l2.83-2.83a5 5 0 1 0-7.07-7.07L11 6" /> <path d="M14 10a5 5 0 0 0-7.07 0L4.1 12.83a5 5 0 0 0 7.07 7.07L13 18" /> </svg>"###
            }
            Self::RecordingCam => {
                r###"<svg width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="recording-icon-cam" id="recordingIconCam" viewBox="0 0 24 24" aria-hidden="true"> <path d="M23 19a2 2 0 0 1-2 2H3a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4l2-3h6l2 3h4a2 2 0 0 1 2 2z"></path> <circle cx="12" cy="13" r="4"></circle> </svg>"###
            }
            Self::RecordingStopSquare => {
                r###"<svg class="recording-stop-square" id="recordingStopSquare" viewBox="0 0 24 24" fill="currentColor"> <rect x="5" y="5" width="14" height="14" rx="2"></rect> </svg>"###
            }
            Self::RecordingStopFolder => {
                r###"<svg class="recording-stop-folder" id="recordingStopFolder" viewBox="0 0 24 24" fill="currentColor"> <path d="M10 4H4c-1.1 0-1.99.9-1.99 2L2 18c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2h-8l-2-2z"></path> </svg>"###
            }
            Self::RecordingSavedFolder => {
                r###"<svg width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="recording-icon-folder" id="recordingIconFolder" viewBox="0 0 24 24" aria-hidden="true"> <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"></path> </svg>"###
            }
            Self::UnlockPadlock => {
                r###"<svg viewBox="0 0 24 24" width="18" height="18" fill="currentColor"> <path d="M12 1a5 5 0 0 0-5 5v3H6a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-9a2 2 0 0 0-2-2h-7V6a3 3 0 1 1 6 0v1h2V6a5 5 0 0 0-5-5zm-6 10h12v9H6v-9z" /> </svg>"###
            }
            Self::CtrlPrev => {
                r###"<svg viewBox="0 0 24 24" width="22" height="22" fill="currentColor"> <path d="M6 6h2v12H6zm3.5 6l8.5 6V6z" /> </svg>"###
            }
            Self::CtrlPlay => {
                r###"<svg class="icon-play" viewBox="0 0 24 24" width="32" height="32" fill="currentColor"> <path d="M8 5v14l11-7z" /> </svg>"###
            }
            Self::CtrlPause => {
                r###"<svg class="icon-pause" viewBox="0 0 24 24" width="32" height="32" fill="currentColor"> <path d="M6 19h4V5H6v14zm8-14v14h4V5h-4z" /> </svg>"###
            }
            Self::CtrlNext => {
                r###"<svg viewBox="0 0 24 24" width="22" height="22" fill="currentColor"> <path d="M6 18l8.5-6L6 6v12zM16 6v12h2V6h-2z" /> </svg>"###
            }
            Self::CtrlVolumeHigh => {
                r###"<svg class="icon-vol-high" viewBox="0 0 24 24" width="22" height="22" fill="currentColor"> <path d="M3 9v6h4l5 5V4L7 9H3zm13.5 3c0-1.77-1.02-3.29-2.5-4.03v8.05c1.48-.73 2.5-2.25 2.5-4.02zM14 3.23v2.06c2.89.86 5 3.54 5 6.71s-2.11 5.85-5 6.71v2.06c4.01-.91 7-4.49 7-8.77s-2.99-7.86-7-8.77z" /> </svg>"###
            }
            Self::CtrlVolumeLow => {
                r###"<svg class="icon-vol-low" viewBox="0 0 24 24" width="22" height="22" fill="currentColor"> <path d="M18.5 12c0-1.77-1.02-3.29-2.5-4.03v8.05c1.48-.73 2.5-2.25 2.5-4.02zM5 9v6h4l5 5V4L9 9H5z" /> </svg>"###
            }
            Self::CtrlVolumeMute => {
                r###"<svg class="icon-vol-mute" viewBox="0 0 24 24" width="22" height="22" fill="currentColor"> <path d="M16.5 12c0-1.77-1.02-3.29-2.5-4.03v2.21l2.45 2.45c.03-.2.05-.41.05-.63zm2.5 0c0 .94-.2 1.82-.54 2.64l1.51 1.51C20.63 14.91 21 13.5 21 12c0-4.28-2.99-7.86-7-8.77v2.06c2.89.86 5 3.54 5 6.71zM4.27 3L3 4.27 7.73 9H3v6h4l5 5v-6.73l4.25 4.25c-.67.52-1.42.93-2.25 1.18v2.06c1.38-.31 2.63-.95 3.69-1.81L19.73 21 21 19.73l-9-9L4.27 3zM12 4L9.91 6.09 12 8.18V4z" /> </svg>"###
            }
            Self::CtrlBgSparkles => {
                r###"<svg width="22" height="22" viewBox="0 0 24 24" fill="currentColor"> <path d="M12 2.5l1.6 6.1 6.1 1.6-6.1 1.6L12 18l-1.6-6.1L4.3 10.2l6.1-1.6z" /> <path d="M18.5 15l.7 2.6 2.6.7-2.6.7-.7 2.6-.7-2.6-2.6-.7 2.6-.7z" /> </svg>"###
            }
            Self::CtrlEqualizer => {
                r###"<svg viewBox="0 0 24 24" width="22" height="22" fill="currentColor"> <path d="M10 20h4V4h-4v16zm-6 0h4v-8H4v8zM16 9v11h4V9h-4z" /> </svg>"###
            }
            Self::CtrlAudioTrack => {
                r###"<svg viewBox="0 0 24 24" width="22" height="22" fill="currentColor"> <path d="M12 3a9 9 0 0 0-9 9v7c0 1.1.9 2 2 2h4v-8H5v-1a7 7 0 0 1 14 0v1h-4v8h4c1.1 0 2-.9 2-2v-7a9 9 0 0 0-9-9z" /> </svg>"###
            }
            Self::CtrlSubtitles => {
                r###"<svg viewBox="0 0 24 24" width="22" height="22" fill="currentColor"> <path d="M20 4H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V6c0-1.1-.9-2-2-2zm0 14H4V6h16v12zM6 10h2v2H6v-2zm0 4h8v2H6v-2zm10 0h2v2h-2v-2zm-6-4h8v2h-8v-2z" /> </svg>"###
            }
            Self::CtrlPlaylist => {
                r###"<svg viewBox="0 0 24 24" width="20" height="20" fill="currentColor"> <path d="M3 13h2v-2H3v2zm0 4h2v-2H3v2zm0-8h2V7H3v2zm4 4h14v-2H7v2zm0 4h14v-2H7v2zM7 7v2h14V7H7z" /> </svg>"###
            }
            Self::CtrlRepeat => {
                r###"<svg class="icon-repeat" viewBox="0 0 438 194" width="24" height="12" fill="currentColor"> <path class="repeat-infinity-path" d="M 408.0 39.0 L 416.0 50.0 L 425.0 70.0 L 427.0 79.0 L 427.0 112.0 L 420.0 133.0 L 411.0 148.0 L 398.0 162.0 L 387.0 170.0 L 372.0 178.0 L 355.0 183.0 L 316.0 182.0 L 298.0 176.0 L 281.0 167.0 L 246.0 138.0 L 238.0 130.0 L 238.0 110.0 L 248.0 101.0 L 250.0 101.0 L 285.0 133.0 L 302.0 145.0 L 317.0 152.0 L 330.0 155.0 L 349.0 154.0 L 369.0 145.0 L 384.0 130.0 L 393.0 111.0 L 393.0 83.0 L 383.0 63.0 L 366.0 48.0 L 346.0 41.0 L 323.0 42.0 L 297.0 52.0 L 279.0 65.0 L 238.0 101.0 L 238.0 60.0 L 276.0 33.0 L 303.0 18.0 L 317.0 13.0 L 332.0 10.0 L 354.0 10.0 L 371.0 14.0 L 385.0 20.0 L 398.0 29.0 L 407.0 38.0 Z M 92.0 41.0 L 79.0 44.0 L 63.0 53.0 L 51.0 67.0 L 46.0 77.0 L 43.0 89.0 L 44.0 109.0 L 51.0 126.0 L 60.0 137.0 L 74.0 147.0 L 92.0 153.0 L 107.0 153.0 L 118.0 151.0 L 134.0 145.0 L 155.0 131.0 L 199.0 92.0 L 199.0 134.0 L 176.0 155.0 L 157.0 168.0 L 138.0 177.0 L 116.0 183.0 L 79.0 183.0 L 64.0 179.0 L 41.0 166.0 L 25.0 149.0 L 15.0 131.0 L 10.0 112.0 L 11.0 79.0 L 16.0 63.0 L 25.0 47.0 L 41.0 30.0 L 61.0 18.0 L 78.0 13.0 L 101.0 12.0 L 121.0 16.0 L 144.0 27.0 L 199.0 70.0 L 199.0 83.0 L 188.0 92.0 L 160.0 67.0 L 143.0 55.0 L 115.0 43.0 L 93.0 41.0 Z M 199 70 Q 219 78 238 60 L 238 138 Q 219 130 199 134 Z" /> <path class="repeat-one-path" d="M 231.0 15.0 L 231.0 154.0 L 234.0 165.0 L 239.0 169.0 L 248.0 170.0 L 249.0 176.0 L 188.0 176.0 L 188.0 171.0 L 200.0 168.0 L 204.0 164.0 L 206.0 157.0 L 206.0 46.0 L 205.0 42.0 L 201.0 38.0 L 186.0 39.0 L 186.0 33.0 L 205.0 27.0 L 225.0 15.0 L 230.0 15.0 Z M 408.0 39.0 L 416.0 50.0 L 425.0 70.0 L 427.0 79.0 L 427.0 112.0 L 420.0 133.0 L 411.0 148.0 L 398.0 162.0 L 387.0 170.0 L 372.0 178.0 L 355.0 183.0 L 316.0 182.0 L 298.0 176.0 L 281.0 167.0 L 246.0 138.0 L 238.0 130.0 L 238.0 110.0 L 248.0 101.0 L 250.0 101.0 L 285.0 133.0 L 302.0 145.0 L 317.0 152.0 L 330.0 155.0 L 349.0 154.0 L 369.0 145.0 L 384.0 130.0 L 393.0 111.0 L 393.0 83.0 L 383.0 63.0 L 366.0 48.0 L 346.0 41.0 L 323.0 42.0 L 297.0 52.0 L 279.0 65.0 L 238.0 101.0 L 238.0 60.0 L 276.0 33.0 L 303.0 18.0 L 317.0 13.0 L 332.0 10.0 L 354.0 10.0 L 371.0 14.0 L 385.0 20.0 L 398.0 29.0 L 407.0 38.0 Z M 92.0 41.0 L 79.0 44.0 L 63.0 53.0 L 51.0 67.0 L 46.0 77.0 L 43.0 89.0 L 44.0 109.0 L 51.0 126.0 L 60.0 137.0 L 74.0 147.0 L 92.0 153.0 L 107.0 153.0 L 118.0 151.0 L 134.0 145.0 L 155.0 131.0 L 199.0 92.0 L 199.0 134.0 L 176.0 155.0 L 157.0 168.0 L 138.0 177.0 L 116.0 183.0 L 79.0 183.0 L 64.0 179.0 L 41.0 166.0 L 25.0 149.0 L 15.0 131.0 L 10.0 112.0 L 11.0 79.0 L 16.0 63.0 L 25.0 47.0 L 41.0 30.0 L 61.0 18.0 L 78.0 13.0 L 101.0 12.0 L 121.0 16.0 L 144.0 27.0 L 199.0 70.0 L 199.0 83.0 L 188.0 92.0 L 160.0 67.0 L 143.0 55.0 L 115.0 43.0 L 93.0 41.0 Z" style="display:none;" /> </svg>"###
            }
            Self::CtrlShuffle => {
                r###"<svg viewBox="0 0 24 24" width="20" height="20" fill="currentColor"> <path d="M10.59 9.17 5.41 4 4 5.41l5.17 5.18 1.42-1.42zM14.5 4l2.04 2.04L4 18.59 5.41 20 17.96 7.46 20 9.5V4h-5.5zm.33 9.41-1.41 1.41 3.13 3.14L14.5 20H20v-5.5l-2.04 2.04-3.13-3.13z" /> </svg>"###
            }
            Self::CtrlAbLoop => {
                r###"<svg class="icon-ab-loop" viewBox="0 0 24 24" width="20" height="20" fill="currentColor"> <path d="M7 7h10v3l4-4-4-4v3H5v6h2V7zm10 10H7v-3l-4 4 4 4v-3h12v-6h-2v4z" /> <text class="ab-loop-badge" x="12" y="14.8" text-anchor="middle" font-size="8.5" font-weight="900" font-family="system-ui, -apple-system, sans-serif" fill="currentColor" style="display:none;"></text> </svg>"###
            }
            Self::CtrlLock => {
                r###"<svg viewBox="0 0 24 24" width="22" height="22" fill="currentColor"> <path d="M17 8h-1V6a4 4 0 1 0-8 0v2H7a2 2 0 0 0-2 2v10a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V10a2 2 0 0 0-2-2zm-7-2a2 2 0 1 1 4 0v2h-4V6zm7 14H7V10h10v10z" /> </svg>"###
            }
            Self::CtrlSettings => {
                r###"<svg viewBox="0 0 24 24" width="22" height="22" fill="currentColor"> <path d="M19.14,12.94c0.04-0.3,0.06-0.61,0.06-0.94c0-0.32-0.02-0.64-0.07-0.94l2.03-1.58c0.18-0.14,0.23-0.41,0.12-0.61 l-1.92-3.32c-0.12-0.22-0.37-0.29-0.59-0.22l-2.39,0.96c-0.5-0.38-1.03-0.7-1.62-0.94L14.4,2.81c-0.04-0.24-0.24-0.41-0.48-0.41 h-3.84c-0.24,0-0.43,0.17-0.47,0.41L9.25,5.35C8.66,5.59,8.12,5.92,7.63,6.29L5.24,5.33c-0.22-0.08-0.47,0-0.59,0.22L2.74,8.87 C2.62,9.08,2.66,9.34,2.86,9.48l2.03,1.58C4.84,11.36,4.8,11.69,4.8,12s0.02,0.64,0.07,0.94l-2.03,1.58 c-0.18,0.14-0.23,0.41-0.12,0.61l1.92,3.32c0.12,0.22,0.37,0.29,0.59,0.22l2.39-0.96c0.5,0.38,1.03,0.7,1.62,0.94l0.36,2.54 c0.05,0.24,0.24,0.41,0.48,0.41h3.84c0.24,0,0.44-0.17,0.47-0.41l0.36-2.54c0.59-0.24,1.13-0.56,1.62-0.94l2.39,0.96 c0.22,0.08,0.47,0,0.59-0.22l1.92-3.32c0.12-0.22,0.07-0.47-0.12-0.61L19.14,12.94z M12,15.6c-1.98,0-3.6-1.62-3.6-3.6 s1.62-3.6,3.6-3.6s3.6,1.62,3.6,3.6S13.98,15.6,12,15.6z" /> </svg>"###
            }
            Self::CtrlFullscreenEnter => {
                r###"<svg class="icon-fullscreen-enter" viewBox="0 0 24 24" width="22" height="22" fill="currentColor"> <path d="M7 14H5v5h5v-2H7v-3zm-2-4h2V7h3V5H5v5zm12 7h-3v2h5v-5h-2v3zM14 5v2h3v3h2V5h-5z" /> </svg>"###
            }
            Self::CtrlFullscreenExit => {
                r###"<svg class="icon-fullscreen-exit" viewBox="0 0 24 24" width="22" height="22" fill="currentColor"> <path d="M5 16h3v3h2v-5H5v2zm3-8H5v2h5V5H8v3zm6 11h2v-3h3v-2h-5v5zm2-11V5h-2v5h5V8h-3z" /> </svg>"###
            }
            Self::PlaylistClose => {
                r###"<svg viewBox="0 0 24 24" width="18" height="18" fill="currentColor"> <path d="M10 6L8.59 7.41 13.17 12l-4.58 4.59L10 18l6-6z" /> </svg>"###
            }
            Self::PlaylistAddFiles => {
                r###"<svg viewBox="0 0 24 24" width="18" height="18" fill="currentColor"> <path d="M19 13h-6v6h-2v-6H5v-2h6V5h2v6h6v2z" /> </svg>"###
            }
            Self::PlaylistShuffle => {
                r###"<svg viewBox="0 0 24 24" width="18" height="18" fill="currentColor"> <path d="M10.59 9.17 5.41 4 4 5.41l5.17 5.18 1.42-1.42zM14.5 4l2.04 2.04L4 18.59 5.41 20 17.96 7.46 20 9.5V4h-5.5zm.33 9.41-1.41 1.41 3.13 3.14L14.5 20H20v-5.5l-2.04 2.04-3.13-3.13z" /> </svg>"###
            }
            Self::PlaylistRepeat => {
                r###"<svg class="icon-repeat" viewBox="0 0 438 194" width="22" height="11" fill="currentColor"> <path class="repeat-infinity-path" d="M 408.0 39.0 L 416.0 50.0 L 425.0 70.0 L 427.0 79.0 L 427.0 112.0 L 420.0 133.0 L 411.0 148.0 L 398.0 162.0 L 387.0 170.0 L 372.0 178.0 L 355.0 183.0 L 316.0 182.0 L 298.0 176.0 L 281.0 167.0 L 246.0 138.0 L 238.0 130.0 L 238.0 110.0 L 248.0 101.0 L 250.0 101.0 L 285.0 133.0 L 302.0 145.0 L 317.0 152.0 L 330.0 155.0 L 349.0 154.0 L 369.0 145.0 L 384.0 130.0 L 393.0 111.0 L 393.0 83.0 L 383.0 63.0 L 366.0 48.0 L 346.0 41.0 L 323.0 42.0 L 297.0 52.0 L 279.0 65.0 L 238.0 101.0 L 238.0 60.0 L 276.0 33.0 L 303.0 18.0 L 317.0 13.0 L 332.0 10.0 L 354.0 10.0 L 371.0 14.0 L 385.0 20.0 L 398.0 29.0 L 407.0 38.0 Z M 92.0 41.0 L 79.0 44.0 L 63.0 53.0 L 51.0 67.0 L 46.0 77.0 L 43.0 89.0 L 44.0 109.0 L 51.0 126.0 L 60.0 137.0 L 74.0 147.0 L 92.0 153.0 L 107.0 153.0 L 118.0 151.0 L 134.0 145.0 L 155.0 131.0 L 199.0 92.0 L 199.0 134.0 L 176.0 155.0 L 157.0 168.0 L 138.0 177.0 L 116.0 183.0 L 79.0 183.0 L 64.0 179.0 L 41.0 166.0 L 25.0 149.0 L 15.0 131.0 L 10.0 112.0 L 11.0 79.0 L 16.0 63.0 L 25.0 47.0 L 41.0 30.0 L 61.0 18.0 L 78.0 13.0 L 101.0 12.0 L 121.0 16.0 L 144.0 27.0 L 199.0 70.0 L 199.0 83.0 L 188.0 92.0 L 160.0 67.0 L 143.0 55.0 L 115.0 43.0 L 93.0 41.0 Z M 199 70 Q 219 78 238 60 L 238 138 Q 219 130 199 134 Z" /> <path class="repeat-one-path" d="M 231.0 15.0 L 231.0 154.0 L 234.0 165.0 L 239.0 169.0 L 248.0 170.0 L 249.0 176.0 L 188.0 176.0 L 188.0 171.0 L 200.0 168.0 L 204.0 164.0 L 206.0 157.0 L 206.0 46.0 L 205.0 42.0 L 201.0 38.0 L 186.0 39.0 L 186.0 33.0 L 205.0 27.0 L 225.0 15.0 L 230.0 15.0 Z M 408.0 39.0 L 416.0 50.0 L 425.0 70.0 L 427.0 79.0 L 427.0 112.0 L 420.0 133.0 L 411.0 148.0 L 398.0 162.0 L 387.0 170.0 L 372.0 178.0 L 355.0 183.0 L 316.0 182.0 L 298.0 176.0 L 281.0 167.0 L 246.0 138.0 L 238.0 130.0 L 238.0 110.0 L 248.0 101.0 L 250.0 101.0 L 285.0 133.0 L 302.0 145.0 L 317.0 152.0 L 330.0 155.0 L 349.0 154.0 L 369.0 145.0 L 384.0 130.0 L 393.0 111.0 L 393.0 83.0 L 383.0 63.0 L 366.0 48.0 L 346.0 41.0 L 323.0 42.0 L 297.0 52.0 L 279.0 65.0 L 238.0 101.0 L 238.0 60.0 L 276.0 33.0 L 303.0 18.0 L 317.0 13.0 L 332.0 10.0 L 354.0 10.0 L 371.0 14.0 L 385.0 20.0 L 398.0 29.0 L 407.0 38.0 Z M 92.0 41.0 L 79.0 44.0 L 63.0 53.0 L 51.0 67.0 L 46.0 77.0 L 43.0 89.0 L 44.0 109.0 L 51.0 126.0 L 60.0 137.0 L 74.0 147.0 L 92.0 153.0 L 107.0 153.0 L 118.0 151.0 L 134.0 145.0 L 155.0 131.0 L 199.0 92.0 L 199.0 134.0 L 176.0 155.0 L 157.0 168.0 L 138.0 177.0 L 116.0 183.0 L 79.0 183.0 L 64.0 179.0 L 41.0 166.0 L 25.0 149.0 L 15.0 131.0 L 10.0 112.0 L 11.0 79.0 L 16.0 63.0 L 25.0 47.0 L 41.0 30.0 L 61.0 18.0 L 78.0 13.0 L 101.0 12.0 L 121.0 16.0 L 144.0 27.0 L 199.0 70.0 L 199.0 83.0 L 188.0 92.0 L 160.0 67.0 L 143.0 55.0 L 115.0 43.0 L 93.0 41.0 Z" style="display:none;" /> </svg>"###
            }
            Self::PlaylistClear => {
                r###"<svg viewBox="0 0 24 24" width="18" height="18" fill="currentColor"> <path d="M6 19c0 1.1.9 2 2 2h8c1.1 0 2-.9 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z" /> </svg>"###
            }
            Self::ModalCloseEqualizer => {
                r###"<svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"> <line x1="1.5" y1="1.5" x2="10.5" y2="10.5" /> <line x1="10.5" y1="1.5" x2="1.5" y2="10.5" /> </svg>"###
            }
            Self::ModalCloseSpeed => {
                r###"<svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"> <line x1="1.5" y1="1.5" x2="10.5" y2="10.5" /> <line x1="10.5" y1="1.5" x2="1.5" y2="10.5" /> </svg>"###
            }
            Self::ModalCloseSubtitle => {
                r###"<svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"> <line x1="1.5" y1="1.5" x2="10.5" y2="10.5" /> <line x1="10.5" y1="1.5" x2="1.5" y2="10.5" /> </svg>"###
            }
            Self::ModalCloseSettings => {
                r###"<svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"> <line x1="1.5" y1="1.5" x2="10.5" y2="10.5" /> <line x1="10.5" y1="1.5" x2="1.5" y2="10.5" /> </svg>"###
            }
            Self::SettingsYoutubeLogo => {
                r###"<svg viewBox="0 0 28 20" class="yt-real-logo" fill="none"> <path d="M27.438 3.125C27.117 1.93 26.176 0.988 24.98 0.668 22.809 0.082 14 0.082 14 0.082s-8.809 0-10.98.586C1.824 0.988 0.883 1.93 0.562 3.125 0 5.309 0 9.844 0 9.844s0 4.535.562 6.719c.321 1.195 1.262 2.137 2.458 2.457C5.191 19.605 14 19.605 14 19.605s8.809 0 10.98-.586c1.196-.32 2.137-1.262 2.458-2.457.562-2.184.562-6.719.562-6.719s0-4.535-.562-6.718z" fill="#FF0000"/> <polygon points="11.2,14.07 18.48,9.844 11.2,5.617" fill="#FFFFFF"/> </svg>"###
            }
            Self::ModalCloseNetworkStream => {
                r###"<svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"> <line x1="1.5" y1="1.5" x2="10.5" y2="10.5" /> <line x1="10.5" y1="1.5" x2="1.5" y2="10.5" /> </svg>"###
            }
            Self::AccordionCheckDither => {
                r###"<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"></polyline></svg>"###
            }
            Self::AccordionCheckParticles => {
                r###"<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"></polyline></svg>"###
            }
            Self::AccordionCheckFaulty => {
                r###"<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"></polyline></svg>"###
            }
            Self::AccordionCheckGridmotion => {
                r###"<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"></polyline></svg>"###
            }
            Self::AccordionCheckNone => {
                r###"<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"></polyline></svg>"###
            }
            Self::ModalCloseBgSettings => {
                r###"<svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"> <line x1="1.5" y1="1.5" x2="10.5" y2="10.5" /> <line x1="10.5" y1="1.5" x2="1.5" y2="10.5" /> </svg>"###
            }
            Self::BgPickerChevron => {
                r###"<svg class="bg-effect-picker-chevron" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="6 9 12 15 18 9"></polyline></svg>"###
            }
            Self::ToastErrorClose => {
                r###"<svg viewBox="0 0 16 16" width="11" height="11" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><line x1="4" y1="4" x2="12" y2="12"/><line x1="12" y1="4" x2="4" y2="12"/></svg>"###
            }
            Self::ToastUpdateBadge => {
                r###"<svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2.3" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>"###
            }
            Self::ToastUpdateAction => {
                r###"<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>"###
            }
            Self::ToastUpdateClose => {
                r###"<svg viewBox="0 0 16 16" width="11" height="11" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><line x1="4" y1="4" x2="12" y2="12"/><line x1="12" y1="4" x2="4" y2="12"/></svg>"###
            }
            Self::ToastSuccessCheck => {
                r###"<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="#22c55e" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>"###
            }
            Self::TitlebarExitFullscreenMonitor => {
                r###"<svg width="12" height="12" viewBox="0 0 12 12"> <rect x="1" y="1" width="10" height="7" rx="1" fill="none" stroke="currentColor" stroke-width="1.2" /> <line x1="6" y1="8" x2="6" y2="10.5" stroke="currentColor" stroke-width="1.2" /> <line x1="3.5" y1="10.5" x2="8.5" y2="10.5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" /> </svg>"###
            }
            Self::TitlebarRestoreStacked => {
                r###"<svg width="12" height="12" viewBox="0 0 12 12"> <path d="M3.5 1.5h7v7" fill="none" stroke="currentColor" stroke-width="1.2" /> <rect x="1.5" y="3.5" width="7" height="7" rx="1" fill="none" stroke="currentColor" stroke-width="1.2" /> </svg>"###
            }
            Self::TitlebarMaximizeSquare => {
                r###"<svg width="12" height="12" viewBox="0 0 12 12"> <rect x="1" y="1" width="10" height="10" rx="1.5" fill="none" stroke="currentColor" stroke-width="1.3" /> </svg>"###
            }
            Self::WelcomeRecentPlay => {
                r###"<svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M8 5v14l11-7z"/></svg>"###
            }
            Self::SkipForwardArrows => {
                r###"<svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor"> <path class="skip-arrow-1" d="M4 6.5v11a1 1 0 001.6.8l7.5-5.5a1 1 0 000-1.6L5.6 5.7A1 1 0 004 6.5z"/> <path class="skip-arrow-2" d="M12 6.5v11a1 1 0 001.6.8l7.5-5.5a1 1 0 000-1.6L13.6 5.7A1 1 0 0012 6.5z"/> </svg>"###
            }
            Self::SkipBackwardArrows => {
                r###"<svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor"> <path class="skip-arrow-2" d="M12 17.5v-11a1 1 0 00-1.6-.8l-7.5 5.5a1 1 0 000 1.6l7.5 5.5a1 1 0 001.6-.8z"/> <path class="skip-arrow-1" d="M20 17.5v-11a1 1 0 00-1.6-.8l-7.5 5.5a1 1 0 000 1.6l7.5 5.5a1 1 0 001.6-.8z"/> </svg>"###
            }
            Self::CarouselPrevArrow => {
                r###"<svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M15 18l-6-6 6-6"/></svg>"###
            }
            Self::CarouselNextArrow => {
                r###"<svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M9 18l6-6-6-6"/></svg>"###
            }
            Self::CustomSelectCheck => {
                r###"<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"></polyline></svg>"###
            }
            Self::ShortcutRowReset => {
                r###"<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 12a9 9 0 1 0 3-6.7L3 8"/><path d="M3 3v5h5"/></svg>"###
            }
            Self::PlaylistEmptyList => {
                r###"<svg viewBox="0 0 24 24" width="40" height="40" fill="currentColor" aria-hidden="true"><path d="M3 13h2v-2H3v2zm0 4h2v-2H3v2zm0-8h2V7H3v2zm4 4h14v-2H7v2zm0 4h14v-2H7v2zM7 7v2h14V7H7z"/></svg>"###
            }
            Self::AuthYoutubeLogo => {
                r###"<svg viewBox="0 0 28 20" class="yt-logo-svg" fill="none"> <path d="M27.438 3.125C27.117 1.93 26.176 0.988 24.98 0.668 22.809 0.082 14 0.082 14 0.082s-8.809 0-10.98.586C1.824 0.988 0.883 1.93 0.562 3.125 0 5.309 0 9.844 0 9.844s0 4.535.562 6.719c.321 1.195 1.262 2.137 2.458 2.457C5.191 19.605 14 19.605 14 19.605s8.809 0 10.98-.586c1.196-.32 2.137-1.262 2.458-2.457.562-2.184.562-6.719.562-6.719s0-4.535-.562-6.718z" fill="#FF0000"/> <polygon points="11.2,14.07 18.48,9.844 11.2,5.617" fill="#FFFFFF"/> </svg>"###
            }
            Self::AuthSecurityShield => {
                r###"<svg viewBox="0 0 20 20" width="12" height="12" fill="currentColor"><path fill-rule="evenodd" d="M10 1.944A11.954 11.954 0 012.166 5C2.056 5.649 2 6.319 2 7c0 5.225 3.34 9.67 8 11.317C14.66 16.67 18 12.225 18 7c0-.682-.057-1.35-.166-2.001A11.954 11.954 0 0110 1.944zM11 14a1 1 0 11-2 0 1 1 0 012 0zm0-7a1 1 0 10-2 0v3a1 1 0 102 0V7z" clip-rule="evenodd"/></svg>"###
            }
            Self::AuthStepDoneCheck => {
                r###"<svg viewBox="0 0 16 16" width="10" height="10" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 8 6.5 11.5 13 4.5"/></svg>"###
            }
        }
    }
}
