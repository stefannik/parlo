//! "Transcribe from URL" end to end through the **real process runner** with `scripts/fake-yt-dlp.py` standing in for
//! yt-dlp: flags, progress/sleep events, every error class, the system-yt-dlp fallback, cancel (process-group kill) and
//! cleanup. No network, no real yt-dlp.
mod common;
use common::{fake_calls, fake_ytdlp};
use parlo_core::audio::prepare::prepare_audio;
use parlo_core::download::ytdlp::YtDlpFetcher;
use parlo_core::download::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

struct Env {
    dir: tempfile::TempDir,
}
impl Env {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }
    fn job(&self) -> PathBuf {
        self.dir.path().join("downloads/dl-0000000000000001")
    }
    fn req(&self, url: &str, opts: DownloadOptions) -> DownloadRequest {
        DownloadRequest {
            url: url.into(),
            job_dir: self.job(),
            cache_dir: self.dir.path().join("cache"),
            opts,
        }
    }
}

fn fetcher(kind: ToolKind, script: &Path) -> Box<dyn Fetcher> {
    Box::new(YtDlpFetcher {
        kind,
        path: script.to_path_buf(),
        deno: None,
    })
}

fn no_sleep() -> DownloadOptions {
    DownloadOptions {
        impersonate: true,
        cookies_browser: None,
        rate_limit_sleep: false,
    }
}

fn run(
    env: &Env,
    url: &str,
    opts: DownloadOptions,
    f: &[Box<dyn Fetcher>],
) -> (Result<Downloaded, DownloadError>, Vec<DownloadEvent>) {
    let mut ev = vec![];
    let r = download(
        &env.req(url, opts),
        f,
        &mut |e| ev.push(e),
        &AtomicBool::new(false),
    );
    (r, ev)
}

fn download_call(calls: &[Vec<String>]) -> Option<&Vec<String>> {
    calls.iter().find(|c| c.iter().any(|a| a == "-f"))
}

#[test]
fn success_downloads_decodes_and_reports_progress() {
    let env = Env::new();
    let fake = fake_ytdlp(env.dir.path(), "bundled");
    let (r, ev) = run(
        &env,
        "https://fake.example/progress/ep1",
        no_sleep(),
        &[fetcher(ToolKind::Bundled, &fake)],
    );
    let d = r.unwrap();
    assert_eq!(d.title, "Fake Episode: Local first");
    assert_eq!(d.duration_s, Some(22.0));
    assert_eq!(d.tool, ToolKind::Bundled);
    assert_eq!(d.path, env.job().join("audio.m4a"));
    assert!(
        matches!(ev.first(), Some(DownloadEvent::Info { title, duration_s: Some(_), playlist: false }) if title.starts_with("Fake Episode"))
    );
    let progress: Vec<_> = ev
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
    assert!(progress.len() >= 2, "{progress:?}");
    assert!(progress.windows(2).all(|w| w[0].0 <= w[1].0), "monotonic");
    assert_eq!(progress.last().unwrap().0, d.bytes);
    assert_eq!(progress.last().unwrap().1, Some(d.bytes));
    assert!(progress.iter().all(|p| p.2 == Some(60000.0)));
    // the pipeline's decoder reads what was downloaded; nothing but the audio + meta.json is left
    let wav = env.dir.path().join("o.wav");
    let p = prepare_audio(&d.path, &wav, &AtomicBool::new(false), |_| {}).unwrap();
    assert!((p.duration_s - common::url_source_secs()).abs() < 0.5);
    let mut left: Vec<_> = std::fs::read_dir(env.job())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(left, vec!["audio.m4a", "meta.json"]);
}

#[test]
fn flags_reach_the_child_as_specified() {
    let env = Env::new();
    let fake = fake_ytdlp(env.dir.path(), "bundled");
    let opts = DownloadOptions {
        impersonate: true,
        cookies_browser: Some("firefox".into()),
        rate_limit_sleep: true,
    };
    let (r, _) = run(
        &env,
        "https://fake.example/success",
        opts,
        &[fetcher(ToolKind::Bundled, &fake)],
    );
    r.unwrap();
    let calls = fake_calls(&fake);
    // --version, --list-impersonate-targets, the -J probe, the download
    assert_eq!(calls.len(), 4, "{calls:?}");
    let probe = calls.iter().find(|c| c.iter().any(|a| a == "-J")).unwrap();
    assert!(
        !probe.iter().any(|a| a == "-f"),
        "the probe downloads nothing"
    );
    let dl = download_call(&calls).unwrap();
    let pair = |k: &str, v: &str| dl.windows(2).any(|w| w[0] == k && w[1] == v);
    assert!(pair("--impersonate", "chrome"));
    assert!(pair("--cookies-from-browser", "firefox"));
    assert!(pair("--sleep-interval", "5") && pair("--max-sleep-interval", "15"));
    assert!(pair("-f", "bestaudio[ext=m4a]/bestaudio"));
    assert!(
        dl.contains(&"--no-config".to_string())
            && dl.contains(&"--no-playlist".to_string())
            && dl.contains(&"--newline".to_string())
    );
    assert!(!dl.iter().any(|a| a == "-x" || a == "--extract-audio"));
    assert_eq!(
        dl[dl.len() - 2..],
        ["--".to_string(), "https://fake.example/success".to_string()]
    );
    let out = dl.windows(2).find(|w| w[0] == "-o").unwrap()[1].clone();
    assert!(
        out.starts_with(env.job().join(".dl").to_str().unwrap()),
        "{out}"
    );
}

#[test]
fn rate_limit_sleep_shows_up_as_a_waiting_event_before_any_progress() {
    let env = Env::new();
    let fake = fake_ytdlp(env.dir.path(), "bundled");
    let opts = DownloadOptions {
        impersonate: false,
        cookies_browser: None,
        rate_limit_sleep: true,
    };
    let (r, ev) = run(
        &env,
        "https://fake.example/sleep",
        opts,
        &[fetcher(ToolKind::Bundled, &fake)],
    );
    r.unwrap();
    let w = ev
        .iter()
        .position(|e| matches!(e, DownloadEvent::Waiting { secs } if *secs == 7.0))
        .expect("waiting event");
    let p = ev
        .iter()
        .position(|e| matches!(e, DownloadEvent::Progress { .. }))
        .unwrap();
    assert!(w < p);
    // toggle off → no sleep flags, no waiting event
    let env2 = Env::new();
    let fake2 = fake_ytdlp(env2.dir.path(), "bundled");
    let (r, ev) = run(
        &env2,
        "https://fake.example/sleep",
        no_sleep(),
        &[fetcher(ToolKind::Bundled, &fake2)],
    );
    r.unwrap();
    assert!(!ev
        .iter()
        .any(|e| matches!(e, DownloadEvent::Waiting { .. })));
    assert!(!download_call(&fake_calls(&fake2))
        .unwrap()
        .iter()
        .any(|a| a.starts_with("--") && a.contains("sleep")));
}

#[test]
fn impersonation_is_skipped_when_the_target_is_missing() {
    let env = Env::new();
    let fake = fake_ytdlp(env.dir.path(), "bundled");
    std::fs::write(fake.with_file_name("no_impersonate"), "").unwrap();
    let (r, _) = run(
        &env,
        "https://fake.example/success",
        no_sleep(),
        &[fetcher(ToolKind::Bundled, &fake)],
    );
    r.unwrap(); // still works
    let calls = fake_calls(&fake);
    assert!(calls
        .iter()
        .any(|c| c.iter().any(|a| a == "--list-impersonate-targets")));
    assert!(
        !calls.iter().any(|c| c.iter().any(|a| a == "--impersonate")),
        "{calls:?}"
    );
}

#[test]
fn live_streams_are_refused_before_anything_is_downloaded() {
    let env = Env::new();
    let fake = fake_ytdlp(env.dir.path(), "bundled");
    let (r, ev) = run(
        &env,
        "https://fake.example/live",
        no_sleep(),
        &[fetcher(ToolKind::Bundled, &fake)],
    );
    assert_eq!(r.unwrap_err().kind, DownloadErrorKind::Live);
    assert!(
        download_call(&fake_calls(&fake)).is_none(),
        "no download call for a live stream"
    );
    assert!(ev.is_empty());
    assert!(!env.job().join("audio.m4a").exists());
}

#[test]
fn every_error_class_has_its_kind_and_a_clean_job_dir() {
    let cases: &[(&str, DownloadErrorKind, &str)] = &[
        (
            "private",
            DownloadErrorKind::NeedsLogin,
            "Use browser cookies",
        ),
        ("age", DownloadErrorKind::NeedsLogin, "age-restricted"),
        ("cookies", DownloadErrorKind::Cookies, "browser’s cookies"),
        ("drm", DownloadErrorKind::Drm, "DRM"),
        (
            "notfound",
            DownloadErrorKind::NotFound,
            "doesn’t seem to exist",
        ),
        ("network", DownloadErrorKind::Network, "internet connection"),
        ("noaudio", DownloadErrorKind::NoAudio, "audio track"),
        ("diskfull", DownloadErrorKind::DiskFull, "disk space"),
        ("live", DownloadErrorKind::Live, "live stream"),
    ];
    for (mode, kind, text) in cases {
        let env = Env::new();
        let fake = fake_ytdlp(env.dir.path(), "bundled");
        let sys = fake_ytdlp(env.dir.path(), "system");
        let (r, _) = run(
            &env,
            &format!("https://fake.example/{mode}"),
            no_sleep(),
            &[
                fetcher(ToolKind::Bundled, &fake),
                fetcher(ToolKind::System, &sys),
            ],
        );
        let e = r.unwrap_err();
        assert_eq!(e.kind, *kind, "{mode}: {e:?}");
        assert!(e.message.contains(text), "{mode}: {}", e.message);
        assert!(
            fake_calls(&sys).is_empty(),
            "{mode}: a verdict about the link must not start the fallback"
        );
        assert!(
            !env.job().join(".dl").exists() && !env.job().join("audio.m4a").exists(),
            "{mode}"
        );
    }
}

#[test]
fn playlist_links_use_only_the_first_item() {
    let env = Env::new();
    let fake = fake_ytdlp(env.dir.path(), "bundled");
    let (r, ev) = run(
        &env,
        "https://fake.example/playlist",
        no_sleep(),
        &[fetcher(ToolKind::Bundled, &fake)],
    );
    assert_eq!(r.unwrap().title, "Fake Episode: Local first");
    assert!(matches!(
        ev.first(),
        Some(DownloadEvent::Info { playlist: true, .. })
    ));
    let dl = download_call(&fake_calls(&fake)).unwrap().clone();
    assert!(
        dl.windows(2).any(|w| w == ["--playlist-items", "1"])
            && dl.contains(&"--no-playlist".to_string())
    );
}

#[test]
fn a_failing_bundled_downloader_retries_with_the_system_one() {
    let env = Env::new();
    let bundled = fake_ytdlp(env.dir.path(), "bundled");
    std::fs::write(bundled.with_file_name("always_extractor_fail"), "").unwrap();
    let system = fake_ytdlp(env.dir.path(), "system");
    let (r, ev) = run(
        &env,
        "https://www.youtube.com/watch?v=x",
        no_sleep(),
        &[
            fetcher(ToolKind::Bundled, &bundled),
            fetcher(ToolKind::System, &system),
        ],
    );
    let d = r.unwrap();
    assert_eq!(d.tool, ToolKind::System);
    let fb = ev.iter().find_map(|e| match e {
        DownloadEvent::Fallback { from, to, .. } => Some((*from, *to)),
        _ => None,
    });
    assert_eq!(fb, Some((ToolKind::Bundled, ToolKind::System)));
    // events from the second tool follow the fallback notice
    let fi = ev
        .iter()
        .position(|e| matches!(e, DownloadEvent::Fallback { .. }))
        .unwrap();
    assert!(ev[fi..]
        .iter()
        .any(|e| matches!(e, DownloadEvent::Info { .. })));
    assert_eq!(
        fake_calls(&system)
            .iter()
            .filter(|c| c.iter().any(|a| a == "-f"))
            .count(),
        1
    );
}

#[test]
fn both_failing_gives_the_install_yt_dlp_message() {
    let env = Env::new();
    let a = fake_ytdlp(env.dir.path(), "bundled");
    let b = fake_ytdlp(env.dir.path(), "system");
    for s in [&a, &b] {
        std::fs::write(s.with_file_name("always_extractor_fail"), "").unwrap();
    }
    let (r, _) = run(
        &env,
        "https://www.youtube.com/watch?v=x",
        no_sleep(),
        &[
            fetcher(ToolKind::Bundled, &a),
            fetcher(ToolKind::System, &b),
        ],
    );
    let e = r.unwrap_err();
    assert_eq!(e.kind, DownloadErrorKind::NoDownloader);
    assert!(e.message.contains("brew install yt-dlp") && e.message.contains("brew upgrade yt-dlp"));
    assert!(e.detail.contains("Unable to extract player response"));
}

#[test]
fn cancel_during_the_download_kills_the_process_group_and_leaves_nothing() {
    let env = Env::new();
    let fake = fake_ytdlp(env.dir.path(), "bundled");
    let cancel = Arc::new(AtomicBool::new(false));
    let c2 = cancel.clone();
    let started = std::time::Instant::now();
    let mut ev_count = 0;
    let r = download(
        &env.req("https://fake.example/hang", no_sleep()),
        &[fetcher(ToolKind::Bundled, &fake)],
        &mut |e| {
            ev_count += 1;
            if matches!(e, DownloadEvent::Progress { .. }) {
                c2.store(true, Ordering::SeqCst); // the user presses Cancel once the first bytes arrived
            }
        },
        &cancel,
    );
    assert_eq!(r.unwrap_err().kind, DownloadErrorKind::Cancelled);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(8),
        "cancel must not wait for the child"
    );
    assert!(ev_count >= 2);
    // the grandchild (`sleep 300`) is gone too
    let pid: i32 = std::fs::read_to_string(fake.with_file_name("child.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(300));
    let zombie = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .map(|s| s.contains(") Z"))
        .unwrap_or(false);
    // SAFETY: signal 0 only probes for existence
    assert!(
        unsafe { libc::kill(pid, 0) } != 0 || zombie,
        "grandchild {pid} survived the cancel"
    );
    assert!(!env.job().join(".dl").exists());
    assert!(!env.job().join("audio.m4a").exists());
}
