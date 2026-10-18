//! The **real** downloaders against a local HTTP server (`scripts/fixture-server.py`): the pinned Linux yt-dlp
//! (`scripts/fetch-tools.sh`; generic extractor) and `/usr/bin/curl` (direct-file path). YouTube etc. can't be reached
//! from CI; this proves the process plumbing, progress parsing, error mapping and decode-ability on real tool output.
mod common;
use common::{fixture, repo_root, FixtureServer};
use parlo_core::audio::prepare::prepare_audio;
use parlo_core::download::curl::CurlFetcher;
use parlo_core::download::ytdlp::{self, YtDlpFetcher};
use parlo_core::download::*;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

fn real_ytdlp() -> PathBuf {
    let p = repo_root().join("tools/yt-dlp/yt-dlp");
    assert!(
        p.exists(),
        "pinned yt-dlp missing at {} — run scripts/fetch-tools.sh",
        p.display()
    );
    p
}

fn ytdlp_only() -> Vec<Box<dyn Fetcher>> {
    vec![Box::new(YtDlpFetcher {
        kind: ToolKind::Bundled,
        path: real_ytdlp(),
        deno: None,
    })]
}
fn curl_only() -> Vec<Box<dyn Fetcher>> {
    vec![Box::new(CurlFetcher {
        path: "/usr/bin/curl".into(),
    })]
}

struct Run {
    _dir: tempfile::TempDir,
    job: PathBuf,
    result: Result<Downloaded, DownloadError>,
    events: Vec<DownloadEvent>,
}

fn go(url: &str, fetchers: &[Box<dyn Fetcher>], sleep: bool) -> Run {
    let dir = tempfile::tempdir().unwrap();
    let job = dir.path().join("downloads/dl-00000000000000aa");
    let req = DownloadRequest {
        url: url.into(),
        job_dir: job.clone(),
        cache_dir: dir.path().join("cache"),
        opts: DownloadOptions {
            impersonate: true,
            cookies_browser: None,
            rate_limit_sleep: sleep,
        },
    };
    let mut events = vec![];
    let result = download(
        &req,
        fetchers,
        &mut |e| events.push(e),
        &AtomicBool::new(false),
    );
    Run {
        _dir: dir,
        job,
        result,
        events,
    }
}

fn assert_decodes(d: &Downloaded) {
    let wav = d.path.with_extension("wav.test");
    let p = prepare_audio(&d.path, &wav, &AtomicBool::new(false), |_| {})
        .unwrap_or_else(|e| panic!("{}: {e}", d.path.display()));
    assert!(
        (p.duration_s - 14.07).abs() < 0.1,
        "{} decoded to {} s",
        d.path.display(),
        p.duration_s
    );
    let _ = std::fs::remove_file(wav);
}

fn server() -> FixtureServer {
    let _ = fixture("generated/url/podcast_ep.mp3"); // fail loudly with the generate hint if missing
    FixtureServer::start(&repo_root().join("fixtures/generated/url"), &[])
}

#[test]
fn real_ytdlp_generic_extractor_downloads_every_audio_shape() {
    let srv = server();
    for (file, ext) in [
        ("podcast_ep.mp3", "mp3"),
        ("episode.m4a", "m4a"),
        ("yt_dash_140.m4a", "m4a"),
        ("yt_dash_251.webm", "webm"),
    ] {
        let r = go(&srv.url(file), &ytdlp_only(), false);
        let d = r.result.unwrap_or_else(|e| panic!("{file}: {e:?}"));
        assert_eq!(d.path, r.job.join(format!("audio.{ext}")), "{file}");
        assert_eq!(d.tool, ToolKind::Bundled);
        assert!(d.bytes > 50_000);
        assert!(
            r.events
                .iter()
                .any(|e| matches!(e, DownloadEvent::Info { .. })),
            "{file}"
        );
        // the title falls back to the file name for a direct link
        assert!(
            matches!(r.events.first(), Some(DownloadEvent::Info { title, .. }) if title.contains(file.split('.').next().unwrap())),
            "{file}: {:?}",
            r.events.first()
        );
        assert_decodes(&d);
        assert!(!r.job.join(".dl").exists());
    }
}

#[test]
fn real_ytdlp_reports_its_rate_limit_sleep() {
    let srv = server();
    let r = go(&srv.url("episode.m4a"), &ytdlp_only(), true);
    r.result.unwrap();
    let waits: Vec<f64> = r
        .events
        .iter()
        .filter_map(|e| match e {
            DownloadEvent::Waiting { secs } => Some(*secs),
            _ => None,
        })
        .collect();
    assert_eq!(waits.len(), 1, "{:?}", r.events);
    assert!(
        (5.0..=15.0).contains(&waits[0]),
        "sleep {} outside 5–15 s",
        waits[0]
    );
}

#[test]
fn real_ytdlp_progress_has_bytes_and_speed_from_a_throttled_server() {
    let _ = fixture("generated/url/podcast_ep.mp3");
    let srv = FixtureServer::start(
        &repo_root().join("fixtures/generated/url"),
        &["--bps", "60000"],
    );
    let r = go(&srv.url("podcast_ep.mp3"), &ytdlp_only(), false);
    let d = r.result.unwrap();
    let prog: Vec<_> = r
        .events
        .iter()
        .filter_map(|e| match e {
            DownloadEvent::Progress {
                downloaded,
                total,
                speed_bps,
                ..
            } => Some((*downloaded, *total, *speed_bps)),
            _ => None,
        })
        .collect();
    assert!(prog.len() >= 3, "{prog:?}");
    assert!(prog.iter().any(|p| p.1 == Some(d.bytes)), "total known");
    assert!(
        prog.iter()
            .any(|p| p.2.is_some_and(|s| s > 10_000.0 && s < 200_000.0)),
        "speed near the 60 kB/s throttle: {prog:?}"
    );
}

#[test]
fn real_ytdlp_capabilities() {
    let caps = ytdlp::capabilities(&real_ytdlp(), true, &AtomicBool::new(false)).unwrap();
    assert_eq!(caps.version, "2026.08.19");
    assert!(caps.supports_js_runtimes());
    assert_eq!(
        caps.impersonate_chrome,
        Some(true),
        "the pinned standalone build ships curl_cffi with Chrome targets"
    );
}

#[test]
fn real_ytdlp_error_mapping() {
    let srv = server();
    let r = go(&srv.url("status/404"), &ytdlp_only(), false);
    assert_eq!(r.result.unwrap_err().kind, DownloadErrorKind::NotFound);
    let r = go("http://127.0.0.1:1/x.mp3", &ytdlp_only(), false);
    let e = r.result.unwrap_err();
    assert_eq!(e.kind, DownloadErrorKind::Network, "{e:?}");
    // an HTML page: yt-dlp has no extractor → generic failure → the "install yt-dlp" message (no system tool, no curl)
    let r = go(&srv.url("html/page"), &ytdlp_only(), false);
    let e = r.result.unwrap_err();
    assert_eq!(e.kind, DownloadErrorKind::NoDownloader);
    assert!(e.detail.contains("Unsupported URL"), "{}", e.detail);
    assert!(!r.job.join("audio.mp3").exists());
    // Spotify is refused up front by yt-dlp itself (DRM) — no network involved
    let r = go("https://open.spotify.com/episode/abc", &ytdlp_only(), false);
    assert_eq!(r.result.unwrap_err().kind, DownloadErrorKind::Drm);
}

#[test]
fn real_curl_downloads_direct_audio_with_redirects_and_without_head() {
    let _ = fixture("generated/url/podcast_ep.mp3");
    let dir = repo_root().join("fixtures/generated/url");
    for extra in [&[][..], &["--no-head"][..]] {
        let srv = FixtureServer::start(&dir, extra);
        for (path, ext) in [
            ("podcast_ep.mp3", "mp3"),
            ("redirect/episode.m4a", "m4a"),
            ("yt_dash_251.webm", "webm"),
        ] {
            let r = go(&srv.url(path), &curl_only(), false);
            let d = r
                .result
                .unwrap_or_else(|e| panic!("{path} {extra:?}: {e:?}"));
            assert_eq!(d.tool, ToolKind::Curl);
            assert_eq!(d.path, r.job.join(format!("audio.{ext}")), "{path}");
            assert_decodes(&d);
            assert!(matches!(r.events.first(), Some(DownloadEvent::Info { .. })));
        }
    }
}

#[test]
fn real_curl_reports_progress() {
    let _ = fixture("generated/url/podcast_ep.mp3");
    let srv = FixtureServer::start(
        &repo_root().join("fixtures/generated/url"),
        &["--bps", "50000"],
    );
    let r = go(&srv.url("podcast_ep.mp3"), &curl_only(), false);
    let d = r.result.unwrap();
    let prog: Vec<_> = r
        .events
        .iter()
        .filter_map(|e| match e {
            DownloadEvent::Progress {
                downloaded, total, ..
            } => Some((*downloaded, *total)),
            _ => None,
        })
        .collect();
    assert!(prog.len() >= 2, "{prog:?}");
    assert!(prog.iter().all(|p| p.0 <= d.bytes));
}

#[test]
fn real_curl_refuses_web_pages_and_maps_errors() {
    let srv = server();
    let r = go(&srv.url("html/page.mp3"), &curl_only(), false); // html served from an .mp3-looking URL: Content-Type wins
    assert_eq!(r.result.unwrap_err().kind, DownloadErrorKind::NoDownloader);
    let r = go(&srv.url("status/404"), &curl_only(), false);
    assert_eq!(r.result.unwrap_err().kind, DownloadErrorKind::NotFound);
    let r = go("http://127.0.0.1:1/x.mp3", &curl_only(), false);
    assert_eq!(r.result.unwrap_err().kind, DownloadErrorKind::Network);
    assert!(!r.job.join("audio.mp3").exists());
}

#[test]
fn chain_yt_dlp_then_curl_serves_a_direct_file_when_yt_dlp_is_missing() {
    // bundled path doesn't exist → the fetcher fails to start (extractor-class) → curl takes over and announces itself
    let srv = server();
    let chain: Vec<Box<dyn Fetcher>> = vec![
        Box::new(YtDlpFetcher {
            kind: ToolKind::Bundled,
            path: "/nonexistent/yt-dlp".into(),
            deno: None,
        }),
        Box::new(CurlFetcher {
            path: "/usr/bin/curl".into(),
        }),
    ];
    let r = go(&srv.url("podcast_ep.mp3"), &chain, false);
    let d = r.result.unwrap();
    assert_eq!(d.tool, ToolKind::Curl);
    assert!(r.events.iter().any(|e| matches!(
        e,
        DownloadEvent::Fallback {
            from: ToolKind::Bundled,
            to: ToolKind::Curl,
            ..
        }
    )));
}

#[test]
fn real_cancel_mid_download_leaves_no_partial_file() {
    let _ = fixture("generated/url/podcast_ep.mp3");
    let srv = FixtureServer::start(
        &repo_root().join("fixtures/generated/url"),
        &["--bps", "15000"],
    );
    for chain in [ytdlp_only(), curl_only()] {
        let dir = tempfile::tempdir().unwrap();
        let job = dir.path().join("downloads/dl-00000000000000bb");
        let req = DownloadRequest {
            url: srv.url("podcast_ep.mp3"),
            job_dir: job.clone(),
            cache_dir: dir.path().join("cache"),
            opts: DownloadOptions {
                impersonate: false,
                cookies_browser: None,
                rate_limit_sleep: false,
            },
        };
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let c2 = cancel.clone();
        let t0 = std::time::Instant::now();
        let r = download(
            &req,
            &chain,
            &mut |e| {
                if matches!(e, DownloadEvent::Progress { downloaded, .. } if downloaded > 0) {
                    c2.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            },
            &cancel,
        );
        assert_eq!(r.unwrap_err().kind, DownloadErrorKind::Cancelled);
        assert!(
            t0.elapsed() < std::time::Duration::from_secs(8),
            "cancel took {:?}",
            t0.elapsed()
        );
        assert!(!job.join(".dl").exists());
        assert!(!job.join("audio.mp3").exists());
    }
}
