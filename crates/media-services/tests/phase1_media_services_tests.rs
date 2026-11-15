use hybrid_media_services::ffmpeg_clip::{
    build_ffmpeg_clip_args, clip_media_segment, format_ffmpeg_timestamp,
    resolve_existing_local_file, ClipError, ClipRequest, ProcessTreeJob, CLIP_EXPORT_TIMEOUT_MS,
    CLIP_MAX_DURATION_SECONDS, CLIP_MAX_START_SECONDS,
};
use hybrid_media_services::youtube_auth::{
    build_netscape_cookie_file, cdp_call, close_process_gracefully,
    describe_browser_import_failure, extract_cookies_from_dedicated_profile, find_chromium_install,
    find_installed_chromium_browser, get_browser_launch_args, get_cookie_file_path,
    get_cookie_file_session_state, get_youtube_dedicated_profile_dir,
    has_authenticated_youtube_cookies, has_youtube_cookie_lines, is_process_alive,
    is_youtube_related_domain, kill_process_tree, sanitize_profile_window_placement,
    sign_out_youtube, sign_out_youtube_in_dir, BrowserImportAttempt, ChromiumBrowser,
    CompanionAuthController, CompanionAuthState, NetscapeCookie, BROWSER_CANDIDATES,
    BROWSER_IMPORT_PROBE_URL, BROWSER_IMPORT_TIMEOUT_MS, SIGNIN_URL,
};
use hybrid_media_services::ytdlp::{
    build_ytdlp_playlist_args, build_ytdlp_video_info_args, extract_youtube_id,
    extract_youtube_playlist_id, extractor_arg_pair, get_youtube_playlist, get_youtube_video_info,
    map_format_to_standard_height, normalize_youtube_url, playlist_items_range_for_url,
    YoutubeInfoCache, YtDlpFormatPayload, DEFAULT_USER_AGENT, YTDLP_EXTRACTOR_ARGS,
    YT_DLP_INFO_CACHE_TTL_MS, YT_DLP_PLAYLIST_MAX_ENTRIES, YT_DLP_PLAYLIST_TIMEOUT_MS,
    YT_DLP_TIMEOUT_MS,
};
use hybrid_media_services::ytdlp_updater::{
    compute_sha256_hex, download_ytdlp_update, extract_tag_from_redirect_location,
    install_binary_with_retry, is_newer_calver, parse_calver, parse_sha256_sums, ReleaseCheckCache,
    UpdateCheckResult, UpdaterError, RELEASE_CHECK_CACHE_MS,
};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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

// ── FFmpeg Lossless Clip ────────────────────────────────────────────────────

#[test]
fn ffmpeg_clip_constants_validation_and_exact_args() {
    assert_eq!(CLIP_EXPORT_TIMEOUT_MS, 5 * 60 * 1000);
    assert!((CLIP_MAX_DURATION_SECONDS - 6.0 * 3600.0).abs() < 1e-6);
    assert!((CLIP_MAX_START_SECONDS - 30.0 * 86_400.0).abs() < 1e-6);

    assert_eq!(format_ffmpeg_timestamp(12.3456), "12.346");
    assert_eq!(format_ffmpeg_timestamp(-5.0), "0.000");

    assert!(resolve_existing_local_file(r"\\server\share\vid.mp4").is_none());
    assert!(resolve_existing_local_file("//server/share/vid.mp4").is_none());

    let dir = TempDirGuard::new("hybrid-clip-args");
    let src = dir.path().join("sample.mkv");
    let out = dir.path().join("sample-clip.mkv");
    fs::write(&src, b"media").unwrap();

    let resolved = resolve_existing_local_file(&src.to_string_lossy()).unwrap();
    assert_eq!(
        fs::canonicalize(&resolved).unwrap(),
        fs::canonicalize(&src).unwrap()
    );

    let args = build_ffmpeg_clip_args(&src, &out, 15.25, 30.5);
    assert_eq!(
        args,
        vec![
            "-hide_banner",
            "-loglevel",
            "error",
            "-protocol_whitelist",
            "file,crypto,data",
            "-ss",
            "15.250",
            "-i",
            &src.to_string_lossy(),
            "-t",
            "30.500",
            "-map",
            "0:v:0?",
            "-map",
            "0:a?",
            "-sn",
            "-dn",
            "-avoid_negative_ts",
            "make_zero",
            "-c",
            "copy",
            &out.to_string_lossy(),
        ]
    );
}

#[tokio::test]
async fn ffmpeg_clip_rejects_limits_runs_mock_and_kills_tree_on_timeout() {
    let dir = TempDirGuard::new("hybrid-clip-exec");
    let src = dir.path().join("movie.mp4");
    fs::write(&src, b"fake video bytes").unwrap();

    // Missing/unsupported/bounds validation
    let err = clip_media_segment(ClipRequest {
        file_path: dir
            .path()
            .join("missing.mp4")
            .to_string_lossy()
            .into_owned(),
        start_time: 0.0,
        duration: 5.0,
        output_dir: Some(dir.path().to_path_buf()),
        ffmpeg_binary: None,
        timeout_ms: None,
    })
    .await
    .unwrap_err();
    assert!(matches!(err, ClipError::InvalidSource));

    let err = clip_media_segment(ClipRequest {
        file_path: src.to_string_lossy().into_owned(),
        start_time: CLIP_MAX_START_SECONDS + 1.0,
        duration: 5.0,
        output_dir: None,
        ffmpeg_binary: None,
        timeout_ms: None,
    })
    .await
    .unwrap_err();
    assert!(matches!(err, ClipError::InvalidStartTime));

    let err = clip_media_segment(ClipRequest {
        file_path: src.to_string_lossy().into_owned(),
        start_time: 0.0,
        duration: CLIP_MAX_DURATION_SECONDS + 1.0,
        output_dir: None,
        ffmpeg_binary: None,
        timeout_ms: None,
    })
    .await
    .unwrap_err();
    assert!(matches!(err, ClipError::DurationTooLong));

    // Mock success ffmpeg (.cmd on Windows, .sh on Unix)
    let args_log = dir.path().join("recorded-args.txt");
    let mock_ok = if cfg!(windows) {
        let p = dir.path().join("mock-ffmpeg-ok.cmd");
        fs::write(
            &p,
            format!(
                "@echo off\r\necho %* > \"{}\"\r\nfor %%A in (%*) do set last=%%~A\r\necho clipdata > \"%last%\"\r\nexit /b 0\r\n",
                args_log.display()
            ),
        )
        .unwrap();
        p
    } else {
        let p = dir.path().join("mock-ffmpeg-ok.sh");
        fs::write(
            &p,
            format!(
                "#!/bin/sh\necho \"$@\" > \"{}\"\nfor last; do :; done\necho clipdata > \"$last\"\n",
                args_log.display()
            ),
        )
        .unwrap();
        p
    };

    let res = clip_media_segment(ClipRequest {
        file_path: src.to_string_lossy().into_owned(),
        start_time: 10.5,
        duration: 20.0,
        output_dir: Some(dir.path().to_path_buf()),
        ffmpeg_binary: Some(mock_ok),
        timeout_ms: Some(5000),
    })
    .await
    .unwrap();
    assert!(res.output_path.exists());
    let logged = fs::read_to_string(&args_log).unwrap();
    assert!(
        logged.contains("-protocol_whitelist") && logged.contains("file,crypto,data"),
        "unexpected logged args: {logged:?}"
    );
    assert!(
        logged.contains("-avoid_negative_ts") && logged.contains("make_zero"),
        "unexpected logged args: {logged:?}"
    );
    assert!(
        logged.contains("-c") && logged.contains("copy"),
        "unexpected logged args: {logged:?}"
    );

    // Timeout + Job Object tree kill + partial output cleanup
    let out_dir = dir.path().join("timeout-out");
    fs::create_dir_all(&out_dir).unwrap();
    let mock_hang = if cfg!(windows) {
        let p = dir.path().join("mock-ffmpeg-hang.cmd");
        fs::write(
            &p,
            "@echo off\r\nfor %%A in (%*) do set last=%%~A\r\necho partial > \"%last%\"\r\nping -n 30 127.0.0.1 > nul\r\nexit /b 0\r\n",
        )
        .unwrap();
        p
    } else {
        let p = dir.path().join("mock-ffmpeg-hang.sh");
        fs::write(&p, "#!/bin/sh\nsleep 30\n").unwrap();
        p
    };

    let timeout_err = clip_media_segment(ClipRequest {
        file_path: src.to_string_lossy().into_owned(),
        start_time: 1.0,
        duration: 2.0,
        output_dir: Some(out_dir.clone()),
        ffmpeg_binary: Some(mock_hang),
        timeout_ms: Some(350),
    })
    .await
    .unwrap_err();
    assert!(matches!(timeout_err, ClipError::Timeout));
    // Verify partial output file was cleaned up
    assert_eq!(fs::read_dir(&out_dir).unwrap().count(), 0);
}

#[cfg(windows)]
#[test]
fn win32_process_tree_job_terminates_child_cleanly() {
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

    let job = ProcessTreeJob::new().expect("CreateJobObjectW must succeed");
    let mut child = StdCommand::new("cmd")
        .args(["/c", "ping -n 30 127.0.0.1 > nul"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let pid = child.id();
    assert!(unsafe { job.assign_process(child.as_raw_handle() as _) });
    assert!(is_process_alive(pid));
    job.terminate_tree();
    let _ = child.wait();
    assert!(!is_process_alive(pid));
}

// ── yt-dlp Probe & Cache ────────────────────────────────────────────────────

#[test]
fn ytdlp_url_extraction_args_and_cache() {
    assert_eq!(YTDLP_EXTRACTOR_ARGS, "");
    assert!(extractor_arg_pair().is_empty());
    assert_eq!(YT_DLP_TIMEOUT_MS, 20_000);
    assert_eq!(YT_DLP_PLAYLIST_TIMEOUT_MS, 45_000);
    assert_eq!(YT_DLP_PLAYLIST_MAX_ENTRIES, 300);
    assert_eq!(YT_DLP_INFO_CACHE_TTL_MS, 30 * 60 * 1000);

    assert_eq!(
        extract_youtube_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ").as_deref(),
        Some("dQw4w9WgXcQ")
    );
    assert_eq!(
        extract_youtube_id("https://youtu.be/BaW_jenozKc").as_deref(),
        Some("BaW_jenozKc")
    );
    assert_eq!(
        extract_youtube_id("https://m.youtube.com/shorts/abcdefghijk").as_deref(),
        Some("abcdefghijk")
    );
    assert!(normalize_youtube_url("https://evil.example.com/watch?v=dQw4w9WgXcQ").is_none());
    assert_eq!(
        extract_youtube_playlist_id(
            "https://www.youtube.com/watch?v=abc12345678&list=PL123456&index=200"
        )
        .as_deref(),
        Some("PL123456")
    );
    assert_eq!(
        playlist_items_range_for_url(
            "https://www.youtube.com/watch?v=abc12345678&list=PL123456&index=200"
        ),
        "50-349"
    );

    let dir = TempDirGuard::new("hybrid-ytdlp-args");
    let cookie_file = dir.path().join("cookies.txt");
    fs::write(&cookie_file, "# Netscape HTTP Cookie File\n").unwrap();

    let args = build_ytdlp_video_info_args(
        "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
        Some(&cookie_file),
    );
    assert!(args.contains(&"--no-warnings".to_string()));
    assert!(args.contains(&DEFAULT_USER_AGENT.to_string()));
    assert!(args.contains(&"--cookies".to_string()));
    assert!(!args.contains(&"--extractor-args".to_string()));

    let p_args = build_ytdlp_playlist_args("PL123456", "1-300", None);
    assert!(p_args.contains(&"--flat-playlist".to_string()));
    assert!(p_args.contains(&"--yes-playlist".to_string()));
    assert!(!p_args.contains(&"--cookies".to_string()));

    // Format height mapping
    assert_eq!(
        map_format_to_standard_height(&YtDlpFormatPayload {
            vcodec: Some("none".into()),
            format_note: Some("audio".into()),
            width: None,
            height: None,
        }),
        None
    );
    assert_eq!(
        map_format_to_standard_height(&YtDlpFormatPayload {
            vcodec: Some("av01".into()),
            format_note: Some("1080p60".into()),
            width: Some(1920.0),
            height: Some(1080.0),
        }),
        Some(1080)
    );
    assert_eq!(
        map_format_to_standard_height(&YtDlpFormatPayload {
            vcodec: Some("vp9".into()),
            format_note: None,
            width: Some(3840.0),
            height: Some(2160.0),
        }),
        Some(2160)
    );
}

#[tokio::test]
async fn ytdlp_mock_probe_populates_30min_cache_and_parses_playlist() {
    let dir = TempDirGuard::new("hybrid-ytdlp-mock");
    let marker = dir.path().join("invocations.txt");
    let mock_bin = if cfg!(windows) {
        let p = dir.path().join("yt-dlp.cmd");
        fs::write(
            &p,
            format!(
                "@echo off\r\necho x >> \"{}\"\r\necho {{\"id\":\"dQw4w9WgXcQ\",\"title\":\"Test Video\",\"uploader\":\"Test Channel\",\"duration\":212,\"formats\":[{{\"vcodec\":\"avc1\",\"format_note\":\"1080p\",\"width\":1920,\"height\":1080}},{{\"vcodec\":\"vp9\",\"format_note\":\"720p\",\"width\":1280,\"height\":720}}]}}\r\n",
                marker.display()
            ),
        )
        .unwrap();
        p
    } else {
        let p = dir.path().join("yt-dlp.sh");
        fs::write(
            &p,
            format!(
                "#!/bin/sh\necho x >> \"{}\"\necho '{{\"id\":\"dQw4w9WgXcQ\",\"title\":\"Test Video\",\"uploader\":\"Test Channel\",\"duration\":212,\"formats\":[{{\"vcodec\":\"avc1\",\"format_note\":\"1080p\",\"width\":1920,\"height\":1080}}]}}'\n",
                marker.display()
            ),
        )
        .unwrap();
        p
    };

    let cache = YoutubeInfoCache::default();
    let info1 = get_youtube_video_info(
        &mock_bin,
        None,
        "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
        Some(&cache),
    )
    .await
    .unwrap();
    assert_eq!(info1.title.as_deref(), Some("Test Video"));
    assert!(info1.heights.contains(&1080));

    // Second lookup must hit cache (still only 1 line in marker file)
    let info2 = get_youtube_video_info(
        &mock_bin,
        None,
        "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
        Some(&cache),
    )
    .await
    .unwrap();
    assert_eq!(info1, info2);
    assert_eq!(fs::read_to_string(&marker).unwrap().lines().count(), 1);

    // TTL expiration check
    let now = Instant::now();
    assert!(cache
        .get_at(
            "dQw4w9WgXcQ",
            now + Duration::from_millis(YT_DLP_INFO_CACHE_TTL_MS + 1)
        )
        .is_none());

    // Mock playlist probe
    let mock_pl = if cfg!(windows) {
        let p = dir.path().join("yt-dlp-pl.cmd");
        fs::write(
            &p,
            "@echo off\r\necho {\"title\":\"My Playlist\",\"playlist_count\":2,\"entries\":[{\"id\":\"dQw4w9WgXcQ\",\"title\":\"One\",\"duration\":60},{\"id\":\"BaW_jenozKc\",\"title\":\"Two\",\"duration\":120}]}\r\n",
        )
        .unwrap();
        p
    } else {
        let p = dir.path().join("yt-dlp-pl.sh");
        fs::write(
            &p,
            "#!/bin/sh\necho '{\"title\":\"My Playlist\",\"playlist_count\":2,\"entries\":[{\"id\":\"dQw4w9WgXcQ\",\"title\":\"One\",\"duration\":60}]}'\n",
        )
        .unwrap();
        p
    };
    let pl = get_youtube_playlist(
        &mock_pl,
        None,
        "https://www.youtube.com/playlist?list=PLabcdef1234",
    )
    .await
    .unwrap();
    assert_eq!(pl.title.as_deref(), Some("My Playlist"));
    assert_eq!(pl.entries.len(), 2);
}

// ── yt-dlp Updater ──────────────────────────────────────────────────────────

#[test]
fn ytdlp_updater_calver_sha_sums_and_15min_cache() {
    assert_eq!(RELEASE_CHECK_CACHE_MS, 15 * 60 * 1000);
    assert_eq!(parse_calver("v2025.03.31"), Some(vec![2025, 3, 31]));
    assert!(is_newer_calver("2025.04.01", "2025.03.31"));
    assert!(!is_newer_calver("2025.03.31", "2025.03.31"));
    assert!(!is_newer_calver("2025.03.30", "2025.03.31"));
    assert!(is_newer_calver("2025.03.31-1", "2025.03.31"));

    assert_eq!(
        extract_tag_from_redirect_location(
            "https://github.com/yt-dlp/yt-dlp/releases/tag/2025.03.31"
        )
        .as_deref(),
        Some("2025.03.31")
    );

    let payload = vec![b'A'; 2048];
    let sha = compute_sha256_hex(&payload);
    let sums = format!("0000 other.exe\n{sha}  *yt-dlp.exe\n");
    assert_eq!(
        parse_sha256_sums(&sums, "yt-dlp.exe").as_deref(),
        Some(sha.as_str())
    );

    let cache = ReleaseCheckCache::default();
    let now = Instant::now();
    cache.set(
        UpdateCheckResult {
            has_update: true,
            current_version: Some("2025.01.01".into()),
            latest_version: Some("2025.03.31".into()),
        },
        now,
    );
    assert!(cache.get(now + Duration::from_secs(60)).is_some());
    assert!(cache
        .get(now + Duration::from_millis(RELEASE_CHECK_CACHE_MS + 1))
        .is_none());
}

#[tokio::test]
async fn ytdlp_updater_downloads_verifies_sha256_and_rejects_mismatch() {
    let dir = TempDirGuard::new("hybrid-updater-http");
    let bin_name = if cfg!(windows) {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    };
    let good_bytes = vec![0x5Au8; 4096];
    let good_sha = compute_sha256_hex(&good_bytes);
    let sums_body = format!("{good_sha}  {bin_name}\n");

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let server_sums = sums_body.clone();
    let server_bytes = good_bytes.clone();
    let handle = std::thread::spawn(move || {
        // Handle 4 HTTP requests: 2 for valid run, 2 for mismatch run
        for req_idx in 0..4 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut req_buf = [0u8; 1024];
            let n = stream.read(&mut req_buf).unwrap_or(0);
            let req_str = String::from_utf8_lossy(&req_buf[..n]);

            if req_str.contains("SHA2-256SUMS") {
                let body = if req_idx >= 2 {
                    format!("{}  {bin_name}\n", "0".repeat(64))
                } else {
                    server_sums.clone()
                };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());
            } else {
                let hdr = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    server_bytes.len()
                );
                let _ = stream.write_all(hdr.as_bytes());
                let _ = stream.write_all(&server_bytes);
            }
        }
    });

    let base_url = format!("http://{addr}/releases/download/2025.03.31");
    let installed = download_ytdlp_update(dir.path(), Some("2025.03.31"), Some(&base_url))
        .await
        .unwrap();
    assert_eq!(fs::read(&installed).unwrap(), good_bytes);

    let bad_err = download_ytdlp_update(dir.path(), Some("2025.03.31"), Some(&base_url))
        .await
        .unwrap_err();
    assert!(matches!(bad_err, UpdaterError::ChecksumMismatch { .. }));

    let _ = handle.join();

    // Retry helper test
    let tmp = dir.path().join("tmp.bin");
    let dst = dir.path().join("final.bin");
    fs::write(&tmp, b"new-binary").unwrap();
    install_binary_with_retry(&tmp, &dst).await.unwrap();
    assert_eq!(fs::read(&dst).unwrap(), b"new-binary");
}

// ── YouTube Option A Auth (ported from youtube-option-a.spec.js) ─────────────

#[test]
fn youtube_auth_netscape_cookies_launch_args_and_signout() {
    assert_eq!(BROWSER_CANDIDATES.len(), 7);
    assert_eq!(
        BROWSER_IMPORT_PROBE_URL,
        "https://www.youtube.com/watch?v=BaW_jenozKc"
    );
    assert_eq!(BROWSER_IMPORT_TIMEOUT_MS, 45_000);
    assert!(SIGNIN_URL.contains("accounts.google.com"));

    let user_data = TempDirGuard::new("hybrid-auth-unit");
    let profile_dir = get_youtube_dedicated_profile_dir(user_data.path());
    assert!(profile_dir
        .to_string_lossy()
        .to_ascii_lowercase()
        .contains("yt-profile"));

    let future_exp = (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 86_400) as i64;
    let sample_cookies = vec![
        NetscapeCookie {
            name: "LOGIN_INFO".into(),
            value: "test-login-token-xyz".into(),
            domain: ".youtube.com".into(),
            host_only: false,
            path: "/".into(),
            secure: true,
            session: false,
            expiration_date: Some(future_exp),
        },
        NetscapeCookie {
            name: "SAPISID".into(),
            value: "test-sapisid-value".into(),
            domain: ".google.com".into(),
            host_only: false,
            path: "/".into(),
            secure: true,
            session: true,
            expiration_date: None,
        },
    ];

    let content = build_netscape_cookie_file(&sample_cookies);
    assert!(content.contains("# Netscape HTTP Cookie File"));
    assert!(has_youtube_cookie_lines(&content));
    assert!(has_authenticated_youtube_cookies(&sample_cookies));

    let lines: Vec<&str> = content
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    assert_eq!(lines.len(), 2);
    for line in &lines {
        let parts: Vec<&str> = line.split('\t').collect();
        assert_eq!(parts.len(), 7);
        assert_eq!(parts[1], "TRUE");
        assert_eq!(parts[2], "/");
        assert_eq!(parts[3], "TRUE");
    }

    // Unauthenticated visitor cookies do not falsely report logged in
    let visitor_cookies = vec![
        NetscapeCookie {
            name: "SOCS".into(),
            value: "CAESEwgDEgk2OTQ0NTIwMTcaAmVuIAEaBgiA_L20Bg".into(),
            domain: ".youtube.com".into(),
            host_only: false,
            path: "/".into(),
            secure: true,
            session: false,
            expiration_date: Some(1800000000),
        },
        NetscapeCookie {
            name: "NID".into(),
            value: "511=test_nid_token".into(),
            domain: ".google.com".into(),
            host_only: false,
            path: "/".into(),
            secure: true,
            session: false,
            expiration_date: Some(1800000000),
        },
    ];
    assert!(!has_authenticated_youtube_cookies(&visitor_cookies));

    // get_browser_launch_args matches exact Option A flags
    let args = get_browser_launch_args(Path::new(r"C:\dummy\profile"), None);
    assert!(args.contains(&r"--user-data-dir=C:\dummy\profile".to_string()));
    assert!(args.contains(&"--no-first-run".to_string()));
    assert!(args.contains(&"--no-default-browser-check".to_string()));
    assert!(args.contains(&"--disable-background-mode".to_string()));
    assert!(args.contains(&"--disable-extensions".to_string()));
    assert!(args.contains(&"--disable-default-apps".to_string()));
    assert!(args.contains(&"--disable-sync".to_string()));
    assert!(!args.contains(&"--disable-component-extensions-with-background-pages".to_string()));
    assert!(args.contains(&"--disable-features=EdgeStartupBoost,Translate".to_string()));
    assert!(args.last().unwrap().contains("accounts.google.com"));

    // sign_out_youtube & get_cookie_file_path ("cookies.txt") + expiry filtering + profile cleanup
    let cookie_path = get_cookie_file_path(user_data.path());
    assert_eq!(
        cookie_path.file_name().and_then(|n| n.to_str()),
        Some("cookies.txt")
    );
    fs::write(&cookie_path, &content).unwrap();
    let state_before = get_cookie_file_session_state(&cookie_path);
    assert!(state_before.exists);
    assert!(state_before.logged_in);
    assert_eq!(state_before.cookie_count, 2);

    // Expired LOGIN_INFO cookie must NOT report logged_in
    let expired_cookies = vec![NetscapeCookie {
        name: "LOGIN_INFO".into(),
        value: "expired-token".into(),
        domain: ".youtube.com".into(),
        host_only: false,
        path: "/".into(),
        secure: true,
        session: false,
        expiration_date: Some(1_000_000_000),
    }];
    fs::write(&cookie_path, build_netscape_cookie_file(&expired_cookies)).unwrap();
    let expired_state = get_cookie_file_session_state(&cookie_path);
    assert!(expired_state.exists);
    assert!(!expired_state.logged_in);
    assert_eq!(expired_state.cookie_count, 0);

    // sign_out_youtube_in_dir cleans cookies.txt and yt-profile/Default/(Network/)Cookies*
    let net_dir = profile_dir.join("Default").join("Network");
    fs::create_dir_all(&net_dir).unwrap();
    let p_cookies = net_dir.join("Cookies");
    let p_journal = profile_dir.join("Default").join("Cookies-journal");
    fs::write(&p_cookies, b"sqlite").unwrap();
    fs::write(&p_journal, b"journal").unwrap();
    assert!(sign_out_youtube_in_dir(user_data.path()).unwrap());
    assert!(!cookie_path.exists());
    assert!(!p_cookies.exists());
    assert!(!p_journal.exists());

    fs::write(&cookie_path, &content).unwrap();
    sign_out_youtube(&cookie_path).unwrap();
    assert!(!cookie_path.exists());
    assert!(!get_cookie_file_session_state(&cookie_path).exists);

    // Profile sanitization cleans crash / window_placement flags
    let prefs_dir = profile_dir.join("Default");
    fs::create_dir_all(&prefs_dir).unwrap();
    fs::write(
        prefs_dir.join("Preferences"),
        r#"{"profile":{"exit_type":"Crashed","exited_cleanly":false},"browser":{"window_placement":{"left":0}}}"#,
    )
    .unwrap();
    sanitize_profile_window_placement(&profile_dir);
    let sanitized: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(prefs_dir.join("Preferences")).unwrap()).unwrap();
    assert_eq!(sanitized["profile"]["exit_type"], "Normal");
    assert_eq!(sanitized["profile"]["exited_cleanly"], true);
    assert!(sanitized["browser"].get("window_placement").is_none());

    // Companion pure-data controller + failure descriptions
    let controller = CompanionAuthController::new();
    let handle = controller.handle();
    controller.set_state(CompanionAuthState::WaitingForSignIn {
        browser: ChromiumBrowser::Chrome,
    });
    assert_eq!(
        handle.state(),
        CompanionAuthState::WaitingForSignIn {
            browser: ChromiumBrowser::Chrome
        }
    );
    handle.request_finish();
    assert!(controller.is_finish_requested());
    assert!(is_youtube_related_domain(".youtube.com"));
    assert!(!is_youtube_related_domain(".evil.example"));

    let locked_msg = describe_browser_import_failure(&[BrowserImportAttempt {
        browser: "edge".into(),
        error: "locked".into(),
        locked: true,
    }]);
    assert!(locked_msg.contains("Edge is currently open"));
}

#[tokio::test]
async fn youtube_auth_cdp_websocket_client_roundtrip_with_mock_server() {
    use tokio_tungstenite::tungstenite::{accept, Message};

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    let server_thread = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut ws = accept(stream).unwrap();
        let req_msg = ws.read().unwrap();
        let req_json: serde_json::Value = serde_json::from_str(req_msg.to_text().unwrap()).unwrap();
        assert_eq!(req_json["method"], "Storage.getCookies");
        let id = req_json["id"].as_u64().unwrap();
        let resp = serde_json::json!({
            "id": id,
            "result": {
                "cookies": [{
                    "name": "LOGIN_INFO",
                    "value": "cdp-mock-token",
                    "domain": ".youtube.com",
                    "path": "/",
                    "secure": true,
                    "session": false,
                    "expires": 1800000000
                }]
            }
        });
        ws.send(Message::Text(resp.to_string().into())).unwrap();
    });

    let result = cdp_call(
        port,
        "/devtools/browser/test-id",
        "Storage.getCookies",
        serde_json::json!({}),
        5000,
    )
    .await
    .unwrap();
    assert_eq!(result["cookies"][0]["name"], "LOGIN_INFO");
    assert_eq!(result["cookies"][0]["value"], "cdp-mock-token");
    let _ = server_thread.join();
}

#[tokio::test]
async fn youtube_auth_headless_cdp_extraction_on_installed_chromium_if_present() {
    let Some(browser) = find_installed_chromium_browser() else {
        // No Chrome/Edge installed on CI runner; unit/mock CDP coverage still ran above.
        return;
    };
    assert!(browser.exe_path.is_file());
    let _ = find_chromium_install(browser.browser);

    let temp_root = TempDirGuard::new("hybrid-test-opt-a-rust");
    let exe = browser.exe_path.clone();

    // Launch headless browser with --remote-debugging-port=0 to verify DevToolsActivePort + graceful close + extraction
    let port_file = temp_root.path().join("DevToolsActivePort");
    let mut child = StdCommand::new(&exe)
        .args([
            &format!("--user-data-dir={}", temp_root.path().display()),
            "--headless",
            "--remote-debugging-port=0",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-gpu",
            "--disable-sync",
            "--disable-background-networking",
            "--disable-extensions",
            "--disable-component-update",
            "--window-size=1,1",
            "about:blank",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let (port, ws_path) =
        hybrid_media_services::youtube_auth::read_devtools_active_port(&port_file, 15_000)
            .await
            .unwrap();

    // Inject synthetic cookie via CDP Storage.setCookies
    let future = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 86_400;
    let _ = cdp_call(
        port,
        &ws_path,
        "Storage.setCookies",
        serde_json::json!({
            "cookies": [{
                "name": "LOGIN_INFO",
                "value": "opt-a-test-value-12345",
                "domain": ".youtube.com",
                "path": "/",
                "secure": true,
                "expires": future
            }]
        }),
        10_000,
    )
    .await
    .unwrap();

    let _ = cdp_call(
        port,
        &ws_path,
        "Browser.close",
        serde_json::json!({}),
        3_000,
    )
    .await;
    close_process_gracefully(child.id(), 2_000);
    kill_process_tree(child.id());
    let _ = child.wait();
    tokio::time::sleep(Duration::from_millis(700)).await;

    let extracted = extract_cookies_from_dedicated_profile(&exe, temp_root.path())
        .await
        .unwrap();
    let injected = extracted
        .iter()
        .find(|c| c.name == "LOGIN_INFO")
        .expect("injected LOGIN_INFO cookie must survive post-exit CDP extraction");
    assert_eq!(injected.value, "opt-a-test-value-12345");
    assert_eq!(injected.domain, ".youtube.com");
    assert!(injected.secure);
}
