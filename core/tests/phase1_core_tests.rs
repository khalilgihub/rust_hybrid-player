use hybrid_core::binary_resolver::{
    is_usable_file, resolve_ffmpeg_binary, resolve_lib_mpv, resolve_mpv_binary,
    resolve_yt_dlp_binary, ResolveOptions,
};
use hybrid_core::db::{
    load_database, save_database_atomic, Database, Preferences, ALL_PREFERENCE_KEYS, HISTORY_LIMIT,
    MEMORY_MAP_LIMIT, RECENT_FILES_LIMIT, SUBTITLE_DELAY_MAX_MS, SUBTITLE_DELAY_MIN_MS,
};
use hybrid_core::equalizer::{
    build_mpv_af_value, generate_filter_string, EqualizerPreset, EqualizerState, EQ_BAND_COUNT,
};
use hybrid_core::media_scanner::{
    is_media_extension, natural_cmp, resolve_screenshot_subfolder, sanitize_windows_name,
    scan_folder_for_media, strip_bracket_tags, MediaScanError, FOLDER_SCAN_MAX_FILES,
    MEDIA_EXTENSIONS,
};
use hybrid_core::playlist::{NavigationAction, PlaylistItem, PlaylistState, RepeatMode};
use hybrid_core::shortcuts::{
    format_combo_compact, format_combo_for_display, normalize_combo, ShortcutManager,
    SHORTCUT_CATALOG, SHORTCUT_CATEGORIES, VALID_SCREENSHOT_MOUSE_BUTTONS,
};
use hybrid_core::tracks::{
    clamp_audio_sync_ms, color_with_alpha, compute_subs_dubs_toggle, next_audio_track,
    next_subtitle_track, normalize_subtitle_sync_sec, MediaTrack, SubsDubsMode, SubtitleStyle,
};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

struct TempDirGuard(PathBuf);

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// ── Database & Persistence ──────────────────────────────────────────────────

#[test]
fn db_load_missing_returns_default_without_error() {
    let dir = TempDirGuard::new("hybrid-db-missing");
    let db_path = dir.path().join("hybrid-player-db.json");
    let res = load_database(&db_path);
    assert!(res.diagnostic.is_none());
    assert!(res.backup_path.is_none());
    assert_eq!(res.db.preferences, Preferences::default());
    assert!(res.db.history.is_empty());
}

#[test]
fn db_corrupt_json_creates_timestamped_backup_and_returns_defaults() {
    let dir = TempDirGuard::new("hybrid-db-corrupt");
    let db_path = dir.path().join("hybrid-player-db.json");
    fs::write(&db_path, "{ not valid json !!!").unwrap();

    let res = load_database(&db_path);
    assert_eq!(res.db.preferences, Preferences::default());
    assert!(res.diagnostic.is_some());
    let backup_path = res.backup_path.expect("expected corrupt backup file");
    let backup_name = backup_path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    assert!(backup_name.starts_with("hybrid-player-db.corrupt-"));
    assert!(backup_path.exists());
    assert!(!db_path.exists());
}

#[test]
fn db_atomic_save_and_reload_roundtrip_all_27_preferences_and_caps() {
    let dir = TempDirGuard::new("hybrid-db-atomic");
    let db_path = dir.path().join("hybrid-player-db.json");

    assert_eq!(ALL_PREFERENCE_KEYS.len(), 27);

    let mut db = Database::default();
    let patch = json!({
        "theme": "light",
        "accentColor": "#ff1122",
        "autoResume": false,
        "brandFontEnabled": false,
        "autoOrganizeScreenshots": false,
        "doubleClickFullscreen": false,
        "showUnlockButton": false,
        "volumeBoost": true,
        "volume": 250.0,
        "subFontSize": 100.0,
        "subBgOpacity": 2.0,
        "subFontColor": "#112233",
        "subBgColor": "#445566",
        "equalizerPreset": "rock",
        "equalizerBands": [12.0, 20.0, -20.0, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
        "motionProfile": "showcase",
        "welcomeBackground": "particles",
        "welcomeQuality": "high",
        "screenshotDir": "C:/Shots",
        "screenshotFormat": "webp",
        "seekStep": 10,
        "customShortcuts": { "fullscreen": "KeyG" },
        "screenshotMouseButton": 4,
        "bgOpts_dither": "{\"speed\":0.5}",
        "bgOpts_particles": "{\"count\":40}",
        "bgOpts_faulty": "{\"scale\":1.2}",
        "bgOpts_gridmotion": "{\"zoom\":1.0}",
        "unknownIgnoredKey": "nope"
    });
    db.preferences.apply_patch(patch.as_object().unwrap());

    assert_eq!(db.preferences.theme, "light");
    assert_eq!(db.preferences.accent_color, "#ff1122");
    assert!(!db.preferences.auto_resume);
    assert!(!db.preferences.brand_font_enabled);
    assert!(!db.preferences.auto_organize_screenshots);
    assert!(!db.preferences.double_click_fullscreen);
    assert!(!db.preferences.show_unlock_button);
    assert!(db.preferences.volume_boost);
    assert!((db.preferences.volume - 2.0).abs() < 1e-6);
    assert!((db.preferences.sub_font_size - 96.0).abs() < 1e-6);
    assert!((db.preferences.sub_bg_opacity - 1.0).abs() < 1e-6);
    assert_eq!(db.preferences.sub_font_color, "#112233");
    assert_eq!(db.preferences.sub_bg_color, "#445566");
    assert_eq!(db.preferences.equalizer_preset, "rock");
    assert!((db.preferences.equalizer_bands[1] - 12.0).abs() < 1e-6);
    assert!((db.preferences.equalizer_bands[2] - (-12.0)).abs() < 1e-6);
    assert_eq!(db.preferences.motion_profile, "showcase");
    assert_eq!(db.preferences.welcome_background, "particles");
    assert_eq!(db.preferences.welcome_quality, "high");
    assert_eq!(db.preferences.screenshot_dir, "C:/Shots");
    assert_eq!(db.preferences.screenshot_format, "webp");
    assert_eq!(db.preferences.seek_step, 10);
    assert_eq!(
        db.preferences
            .custom_shortcuts
            .get("fullscreen")
            .map(String::as_str),
        Some("KeyG")
    );
    assert_eq!(db.preferences.screenshot_mouse_button, 4);
    assert_eq!(
        db.preferences.bg_opts_dither.as_deref(),
        Some("{\"speed\":0.5}")
    );

    // Invalid hex color rejected (retains previous valid value)
    assert!(!db.preferences.set("accentColor", &json!("not-a-color")));
    assert_eq!(db.preferences.accent_color, "#ff1122");

    // Bounded history (100), recent_files (50), memory maps (1000)
    for i in 0..120 {
        db.add_history(
            &format!("C:/video_{i}.mkv"),
            Some(&format!("Video {i}")),
            120.0,
        );
    }
    assert_eq!(db.history.len(), HISTORY_LIMIT);
    assert_eq!(db.history[0].path, "C:/video_119.mkv");
    assert_eq!(db.recent_files.len(), RECENT_FILES_LIMIT);
    assert_eq!(db.recent_files[0], "C:/video_119.mkv");

    for i in 0..1050 {
        db.save_resume_position(&format!("k_{i}"), i as f64);
        db.save_speed(&format!("s_{i}"), 1.25);
        db.save_subtitle_delay(&format!("d_{i}"), i as f64 * 1000.0);
    }
    assert_eq!(db.resume_positions.len(), MEMORY_MAP_LIMIT);
    assert!(db.resume_positions.get("k_0").is_none());
    assert_eq!(db.resume_positions.get("k_1049"), Some(&1049.0));
    assert_eq!(db.speed_memory.len(), MEMORY_MAP_LIMIT);
    assert_eq!(db.subtitle_delay_memory.len(), MEMORY_MAP_LIMIT);
    assert_eq!(
        db.subtitle_delay_memory.get("d_1049"),
        Some(&SUBTITLE_DELAY_MAX_MS)
    );
    db.save_subtitle_delay("neg", -9_999_999.0);
    assert_eq!(
        db.subtitle_delay_memory.get("neg"),
        Some(&SUBTITLE_DELAY_MIN_MS)
    );

    save_database_atomic(&db, &db_path).unwrap();
    let loaded = load_database(&db_path);
    assert!(loaded.diagnostic.is_none());
    assert_eq!(loaded.db.preferences.theme, "light");
    assert_eq!(loaded.db.history.len(), HISTORY_LIMIT);
    assert_eq!(loaded.db.recent_files.len(), RECENT_FILES_LIMIT);
    assert_eq!(loaded.db.resume_positions.len(), MEMORY_MAP_LIMIT);
    assert_eq!(loaded.db.resume_positions.get("k_50"), Some(&50.0));
}

// ── Equalizer ───────────────────────────────────────────────────────────────

#[test]
fn equalizer_18_band_positional_syntax_and_zero_reset() {
    let mut eq = EqualizerState::default();
    assert_eq!(eq.bands.len(), EQ_BAND_COUNT);
    assert_eq!(eq.mpv_af_value(), "");

    // Equal +6 dB across all 10 UI bands -> peak attenuation brings every one of 18 FFmpeg bands to 1.00
    eq.bands = [6.0; EQ_BAND_COUNT];
    let af = eq.mpv_af_value();
    assert!(af.starts_with("lavfi=[superequalizer="));
    assert!(!af.contains("g0:"));
    assert!(!af.contains("b0="));
    let inner = af
        .strip_prefix("lavfi=[superequalizer=")
        .unwrap()
        .strip_suffix(']')
        .unwrap();
    let parts: Vec<&str> = inner.split(':').collect();
    assert_eq!(parts.len(), 18);
    for p in parts {
        assert_eq!(p, "1.00");
    }

    // Extreme cuts clamp at 0.25 (10^(-12/20) = 0.251...) -> within [0.05, 20.0]
    let cut_af = generate_filter_string(&[-12.0; 10]);
    assert!(cut_af.starts_with("lavfi=[superequalizer=0.25:"));

    // 9 builtin presets + custom
    assert_eq!(EqualizerPreset::BUILTIN_PRESETS.len(), 9);
    assert!(eq.apply_preset(EqualizerPreset::BassBoost));
    assert_eq!(eq.preset, EqualizerPreset::BassBoost);
    assert_ne!(build_mpv_af_value(&eq.bands), "");
    eq.set_band(0, 3.0);
    assert_eq!(eq.preset, EqualizerPreset::Custom);
    assert!(eq.apply_preset(EqualizerPreset::Flat));
    assert_eq!(build_mpv_af_value(&eq.bands), "");
}

// ── Shortcuts ───────────────────────────────────────────────────────────────

#[test]
fn shortcuts_catalog_52_entries_6_categories_and_rebinding() {
    assert_eq!(SHORTCUT_CATALOG.len(), 52);
    assert_eq!(SHORTCUT_CATEGORIES.len(), 6);
    assert_eq!(VALID_SCREENSHOT_MOUSE_BUTTONS, [-1, 1, 3, 4]);

    let mut mgr = ShortcutManager::default();
    assert_eq!(mgr.screenshot_mouse_button(), 3);
    assert_eq!(mgr.set_screenshot_mouse_button(2), 3); // invalid retains default
    assert_eq!(mgr.set_screenshot_mouse_button(4), 4);

    assert_eq!(mgr.resolve_action("KeyF"), Some("toggle-fullscreen"));
    // Shift+KeyF fallback alias -> toggle-fullscreen
    assert_eq!(mgr.resolve_action("Shift+KeyF"), Some("toggle-fullscreen"));

    // Conflict displaces existing owner
    let outcome = mgr.set_shortcut("screenshot", "Space").unwrap();
    assert_eq!(outcome.displaced_action_id.as_deref(), Some("toggle-play"));
    assert_eq!(mgr.get_binding("toggle-play"), "");
    assert_eq!(mgr.get_binding("screenshot"), "Space");

    // Lock screen only allows unlock-ui, toggle-lock, screenshot, screenshot-png
    assert!(!ShortcutManager::is_action_allowed_when_locked(
        "toggle-play"
    ));
    assert!(ShortcutManager::is_action_allowed_when_locked(
        "toggle-lock"
    ));
    assert!(ShortcutManager::is_action_allowed_when_locked("unlock-ui"));
    assert!(ShortcutManager::is_action_allowed_when_locked("screenshot"));

    // Roundtrip custom overrides
    let overrides = mgr.get_custom_overrides();
    let mut mgr2 = ShortcutManager::default();
    mgr2.apply_custom_shortcuts(&overrides);
    assert_eq!(mgr2.get_binding("screenshot"), "Space");
    assert_eq!(mgr2.get_binding("toggle-play"), "");

    assert_eq!(normalize_combo("Shift+Equal"), "Shift+Equal");
    assert_eq!(format_combo_for_display("Shift+KeyF"), "Shift + F");
    assert_eq!(format_combo_compact("Ctrl+KeyO"), "Ctrl+O");
}

// ── Tracks & Playlist ───────────────────────────────────────────────────────

#[test]
fn tracks_sync_clamping_style_and_subs_dubs() {
    assert_eq!(clamp_audio_sync_ms(999_999), 600_000);
    assert_eq!(clamp_audio_sync_ms(-999_999), -600_000);
    assert!((normalize_subtitle_sync_sec(1500.0, false) - 1.5).abs() < 1e-6);
    assert!((normalize_subtitle_sync_sec(250.0, true) - 0.25).abs() < 1e-6);
    assert!((normalize_subtitle_sync_sec(900.0, false) - 600.0).abs() < 1e-6);

    assert_eq!(color_with_alpha("#000000", 0.6), "#00000099");
    let mut style = SubtitleStyle::default();
    assert!(!style.has_custom_style());
    style.font_size = 40;
    assert!(style.has_custom_style());

    let audio = vec![
        MediaTrack {
            id: 1,
            lang: "ja".into(),
            title: "Japanese".into(),
            selected: true,
            external: false,
        },
        MediaTrack {
            id: 2,
            lang: "en".into(),
            title: "English Dub".into(),
            selected: false,
            external: false,
        },
    ];
    let subs = vec![
        MediaTrack {
            id: 10,
            lang: "en".into(),
            title: "English Signs & Songs".into(),
            selected: false,
            external: false,
        },
        MediaTrack {
            id: 11,
            lang: "en".into(),
            title: "English Full".into(),
            selected: false,
            external: false,
        },
    ];
    let dubs = compute_subs_dubs_toggle(&audio, &subs, Some(1)).unwrap();
    assert_eq!(dubs.mode, SubsDubsMode::Dubs);
    assert_eq!(dubs.audio_track_id, Some(2));
    assert_eq!(dubs.subtitle_track_id, Some(10));
    assert!(dubs.subtitle_visible);

    let back_to_subs = compute_subs_dubs_toggle(&audio, &subs, Some(2)).unwrap();
    assert_eq!(back_to_subs.mode, SubsDubsMode::Subs);
    assert_eq!(back_to_subs.audio_track_id, Some(1));
    assert_eq!(back_to_subs.subtitle_track_id, Some(11));

    assert_eq!(next_audio_track(&audio, Some(1)).unwrap().id, 2);
    assert_eq!(next_subtitle_track(&subs, Some(10)).unwrap().id, 11);
    assert!(next_subtitle_track(&subs, Some(11)).is_none());
}

#[test]
fn playlist_filters_subs_shuffles_without_repeat_and_restarts_previous() {
    let mut pl = PlaylistState::new();
    let first = pl.add_paths([
        ("C:/vids/ep1.mkv".to_string(), false),
        ("C:/vids/ep1.srt".to_string(), false),
        ("C:/vids/ep2.mkv".to_string(), false),
        ("C:/vids/ep3.mkv".to_string(), false),
    ]);
    assert_eq!(first, Some(0));
    assert_eq!(pl.items.len(), 3);
    pl.set_current_index(0);

    // >3s restarts current; <=3s at index 0 with RepeatMode::None also restarts/seeks to 0 without reload
    assert_eq!(pl.play_previous(4.5), NavigationAction::RestartCurrent);
    assert_eq!(pl.play_previous(1.0), NavigationAction::RestartCurrent);

    pl.set_shuffle(true);
    let mut visited = vec![0usize];
    for _ in 0..2 {
        match pl.play_next(true) {
            NavigationAction::PlayIndex(idx) => visited.push(idx),
            other => panic!("expected PlayIndex, got {other:?}"),
        }
    }
    visited.sort();
    assert_eq!(visited, vec![0, 1, 2]);
    // With repeat::None, both auto_advance and manual next stop once all played
    assert_eq!(pl.play_next(true), NavigationAction::None);
    assert_eq!(pl.play_next(false), NavigationAction::None);

    pl.repeat = RepeatMode::One;
    let cur = pl.current_index as usize;
    assert_eq!(pl.play_next(true), NavigationAction::PlayIndex(cur));

    for i in 0..85 {
        pl.add_item(PlaylistItem::from_path(
            format!("C:/vids/extra_{i}.mp4"),
            false,
        ));
    }
    assert!(pl.should_virtualize());
}

// ── Media Scanner & Folder Resolver ─────────────────────────────────────────

#[test]
fn media_scanner_extensions_unc_natural_sort_and_limits() {
    assert_eq!(MEDIA_EXTENSIONS.len(), 30);
    assert!(is_media_extension(Path::new("movie.MKV")));
    assert!(!is_media_extension(Path::new("notes.txt")));

    let err = scan_folder_for_media(Path::new(r"\\server\share\media")).unwrap_err();
    assert!(matches!(err, MediaScanError::UncPathRejected(_)));
    let err2 = scan_folder_for_media(Path::new("//server/share")).unwrap_err();
    assert!(matches!(err2, MediaScanError::UncPathRejected(_)));

    let dir = TempDirGuard::new("hybrid-scan");
    let sub = dir.path().join("Season 1");
    fs::create_dir_all(&sub).unwrap();
    fs::write(sub.join("ep10.mkv"), b"x").unwrap();
    fs::write(sub.join("ep2.mkv"), b"x").unwrap();
    fs::write(sub.join("ep1.mkv"), b"x").unwrap();
    fs::write(sub.join("ep1.srt"), b"x").unwrap();

    let stats = scan_folder_for_media(dir.path()).unwrap();
    assert_eq!(stats.files.len(), 3);
    let names: Vec<String> = stats
        .files
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    assert_eq!(names, vec!["ep1.mkv", "ep2.mkv", "ep10.mkv"]);
    assert!(!stats.hit_file_limit);
    assert_eq!(FOLDER_SCAN_MAX_FILES, 2000);
    assert!(natural_cmp("ep2", "ep10").is_lt());
}

#[test]
fn media_folder_resolver_all_8_scenarios_plus_dos_reserved() {
    // Scenario 1
    let w = resolve_screenshot_subfolder(r"E:\vid\mob psycho 100 season 1 episode 2.mkv", "");
    let c = resolve_screenshot_subfolder(r"E:\vid\mob psycho 100 S01E03.mkv", "");
    assert_eq!(w, PathBuf::from("Mob Psycho 100").join("Season 1"));
    assert_eq!(w, c);

    // Scenario 2
    assert_eq!(
        resolve_screenshot_subfolder(
            r"E:\vid\[Judas] Mob Psycho 100 (Season 3) [1080p]\[Judas] Mob Psycho 100 - S03E01v2.mkv",
            ""
        ),
        PathBuf::from("Mob Psycho 100").join("Season 3")
    );
    assert_eq!(
        resolve_screenshot_subfolder(
            r"C:\Downloads\[SubsPlease] Mob Psycho 100 - S03E01 (1080p) [A1B2C3D4].mkv",
            ""
        ),
        PathBuf::from("Mob Psycho 100").join("Season 3")
    );

    // Scenario 3
    assert_eq!(
        resolve_screenshot_subfolder(r"E:\vid\Mob Psycho 100 III - 01.mkv", ""),
        PathBuf::from("Mob Psycho 100").join("Season 3")
    );
    assert_eq!(
        resolve_screenshot_subfolder(r"E:\vid\Mob Psycho 100 II - 05.mkv", ""),
        PathBuf::from("Mob Psycho 100").join("Season 2")
    );

    // Scenario 4
    assert_eq!(
        resolve_screenshot_subfolder(
            r"E:\vid\[Judas] Mob Psycho 100 - Consultation Office [OVA].mkv",
            ""
        ),
        PathBuf::from("Mob Psycho 100").join("Specials")
    );

    // Scenario 5
    assert_eq!(
        resolve_screenshot_subfolder(r"D:\Torrents\Mob.Psycho.100.S02E05.1080p.mkv", ""),
        PathBuf::from("Mob Psycho 100").join("Season 2")
    );

    // Scenario 6
    assert_eq!(
        resolve_screenshot_subfolder(r"E:\Movies\Inception (2010).mp4", ""),
        PathBuf::from("Inception (2010)")
    );
    assert_eq!(
        resolve_screenshot_subfolder(r"C:\Users\User\Videos\VID_20260902_142030.mp4", ""),
        PathBuf::new()
    );

    // Scenario 7
    assert_eq!(
        resolve_screenshot_subfolder(
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "YOASOBI「Idol」| Official MV / 4K"
        ),
        PathBuf::from("YouTube").join("YOASOBI「Idol」 - Official MV - 4K")
    );

    // Scenario 8
    assert_eq!(
        resolve_screenshot_subfolder(
            "https://www.youtube.com/watch?v=trailer123",
            "Mob Psycho 100 Season 3 - Official Trailer | Crunchyroll"
        ),
        PathBuf::from("Mob Psycho 100").join("Trailers")
    );

    // DOS reserved device names & bracket stripping
    assert_eq!(sanitize_windows_name("CON"), "CON_");
    assert_eq!(sanitize_windows_name("com9"), "com9_");
    assert_eq!(sanitize_windows_name("LPT1"), "LPT1_");
    assert_eq!(sanitize_windows_name("nul"), "nul_");
    assert_eq!(
        strip_bracket_tags("[Judas] Inception (2010) [1080p] (x265)"),
        "Inception (2010)"
    );
}

// ── Binary Resolver (ported from binary-resolver.spec.js) ───────────────────

#[test]
fn binary_resolver_prefers_bundled_ffmpeg_without_path_lookup() {
    let dir = TempDirGuard::new("hybrid-binary-test-");
    let resources_dir = dir.path().join("resources");
    let exec_dir = dir.path().join("exec");
    let app_dir = dir.path().join("app").join("src").join("main");
    let bin_name = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    let bundled = resources_dir.join("ffmpeg").join(bin_name);

    fs::create_dir_all(bundled.parent().unwrap()).unwrap();
    fs::create_dir_all(&exec_dir).unwrap();
    fs::create_dir_all(&app_dir).unwrap();
    fs::write(&bundled, b"").unwrap();

    let exec_path = exec_dir.join(if cfg!(windows) { "app.exe" } else { "app" });
    let found = resolve_ffmpeg_binary(
        Some(&resources_dir),
        Some(&exec_path),
        Some(&app_dir),
        ResolveOptions {
            allow_path_lookup: false,
        },
    );
    assert_eq!(found.as_deref(), Some(bundled.as_path()));
}

#[test]
fn binary_resolver_resolves_ffmpeg_static_in_dev_without_path_lookup() {
    let dir = TempDirGuard::new("hybrid-binary-test-");
    let resources_dir = dir.path().join("resources");
    let exec_dir = dir.path().join("exec");
    let app_dir = dir.path().join("app").join("src").join("main");
    let project_root = dir.path().join("app");
    let bin_name = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    let static_bin = project_root
        .join("node_modules")
        .join("ffmpeg-static")
        .join(bin_name);

    fs::create_dir_all(&resources_dir).unwrap();
    fs::create_dir_all(&exec_dir).unwrap();
    fs::create_dir_all(static_bin.parent().unwrap()).unwrap();
    fs::write(&static_bin, b"").unwrap();

    let exec_path = exec_dir.join(if cfg!(windows) { "app.exe" } else { "app" });
    let found = resolve_ffmpeg_binary(
        Some(&resources_dir),
        Some(&exec_path),
        Some(&app_dir),
        ResolveOptions {
            allow_path_lookup: false,
        },
    );
    assert_eq!(
        fs::canonicalize(found.unwrap()).unwrap(),
        fs::canonicalize(static_bin).unwrap()
    );
}

#[test]
fn binary_resolver_ytdlp_prefers_user_data_bin_and_enforces_size_guard() {
    let dir = TempDirGuard::new("hybrid-real-size");
    let user_data = dir.path().join("userData");
    let resources_dir = dir.path().join("resources");
    let exec_path = dir.path().join("exec").join("app.exe");
    let app_dir = dir.path().join("app").join("src").join("main");

    let bin_name = if cfg!(windows) {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    };
    let ud_ytdlp = user_data.join("bin").join(bin_name);
    let res_ytdlp = resources_dir.join("mpv").join(bin_name);
    fs::create_dir_all(ud_ytdlp.parent().unwrap()).unwrap();
    fs::create_dir_all(res_ytdlp.parent().unwrap()).unwrap();
    fs::create_dir_all(exec_path.parent().unwrap()).unwrap();
    fs::create_dir_all(&app_dir).unwrap();

    fs::write(&ud_ytdlp, b"tiny").unwrap();
    assert!(!is_usable_file(&ud_ytdlp));
    fs::write(&res_ytdlp, vec![b'x'; 2048]).unwrap();
    let fallback = resolve_yt_dlp_binary(
        Some(&user_data),
        Some(&resources_dir),
        Some(&exec_path),
        Some(&app_dir),
        ResolveOptions {
            allow_path_lookup: false,
        },
    );
    assert_eq!(fallback.as_deref(), Some(res_ytdlp.as_path()));

    fs::write(&ud_ytdlp, vec![b'y'; 2048]).unwrap();
    let preferred = resolve_yt_dlp_binary(
        Some(&user_data),
        Some(&resources_dir),
        Some(&exec_path),
        Some(&app_dir),
        ResolveOptions {
            allow_path_lookup: false,
        },
    );
    assert_eq!(preferred.as_deref(), Some(ud_ytdlp.as_path()));

    let libmpv_name = if cfg!(windows) {
        "libmpv-2.dll"
    } else {
        "libmpv.so"
    };
    let libmpv_path = resources_dir.join("mpv").join(libmpv_name);
    fs::write(&libmpv_path, vec![b'z'; 2048]).unwrap();
    assert_eq!(
        resolve_lib_mpv(Some(&resources_dir), Some(&exec_path), Some(&app_dir)).as_deref(),
        Some(libmpv_path.as_path())
    );
    let _ = resolve_mpv_binary(
        Some(&resources_dir),
        Some(&exec_path),
        Some(&app_dir),
        ResolveOptions {
            allow_path_lookup: false,
        },
    );
}

#[test]
fn binary_resolver_from_env_and_repo_root_app_dir() {
    use hybrid_core::binary_resolver::BinarySearchPaths;

    let search = BinarySearchPaths::from_env();
    assert!(
        search.app_dir.is_some(),
        "expected BinarySearchPaths::from_env() to locate hybrid-player/src/main"
    );

    let dir = TempDirGuard::new("hybrid-binary-test-repo-root");
    let repo_root = dir.path();
    let hybrid_dir = repo_root.join("hybrid-player");
    let libmpv_name = if cfg!(windows) {
        "libmpv-2.dll"
    } else {
        "libmpv.so"
    };
    let dll = hybrid_dir.join("mpv").join(libmpv_name);
    fs::create_dir_all(dll.parent().unwrap()).unwrap();
    fs::write(&dll, vec![1u8; 2048]).unwrap();

    let resolved = resolve_lib_mpv(None, None, Some(repo_root));
    assert_eq!(resolved.as_deref(), Some(dll.as_path()));
}
