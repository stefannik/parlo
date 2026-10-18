//! "Transcribe from URL" — the download stage (DECISIONS C-021, C-022).
//!
//! Principles: the user starts every download; **Parlo itself never opens a socket** — all networking happens in a child
//! process (yt-dlp, or `/usr/bin/curl` for a plain audio file); there is no HTTP client in this crate. Audio only, no
//! ffmpeg, into a per-job directory; the file only appears under its final name when it is complete.
//!
//! Downloaders sit behind the [`Fetcher`] trait (like the STT/TTS engines), so the order of attempts and the
//! fallbacks are unit-tested with fakes, and the flags / parsing without a network.

pub mod curl;
pub mod discover;
pub mod error;
pub mod proc;
pub mod store;
pub mod url;
pub mod ytdlp;

pub use error::{DownloadError, DownloadErrorKind};

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Which downloader did (or will do) the work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Bundled,
    System,
    Curl,
}

impl ToolKind {
    pub fn label(self) -> &'static str {
        match self {
            ToolKind::Bundled => "the bundled downloader",
            ToolKind::System => "system yt-dlp",
            ToolKind::Curl => "curl",
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            ToolKind::Bundled => "bundled",
            ToolKind::System => "system",
            ToolKind::Curl => "curl",
        }
    }
}

/// The user-controlled knobs (Settings → Downloads).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadOptions {
    pub impersonate: bool,
    /// yt-dlp browser name, `None` = cookies off
    pub cookies_browser: Option<String>,
    pub rate_limit_sleep: bool,
}

impl Default for DownloadOptions {
    fn default() -> Self {
        Self {
            impersonate: true,
            cookies_browser: None,
            rate_limit_sleep: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DownloadEvent {
    /// title (and length) known — before any bytes arrive
    Info {
        title: String,
        duration_s: Option<f64>,
        playlist: bool,
    },
    /// yt-dlp's rate-limit sleep before the download
    Waiting { secs: f64 },
    Progress {
        downloaded: u64,
        total: Option<u64>,
        speed_bps: Option<f64>,
        eta_s: Option<f64>,
    },
    /// the previous downloader failed; this one is taking over
    Fallback {
        from: ToolKind,
        to: ToolKind,
        reason: String,
    },
}

pub struct FetchCtx {
    pub url: String,
    /// where the downloader writes `media.<ext>` (+ `.part`)
    pub stage_dir: PathBuf,
    /// scratch space for the child (PyInstaller onefile unpacking …)
    pub tmp_dir: PathBuf,
    /// yt-dlp's cache (signature/player caches), inside Parlo's cache dir
    pub cache_dir: PathBuf,
    pub opts: DownloadOptions,
}

#[derive(Debug, Clone)]
pub struct FetchOut {
    /// complete media file inside `stage_dir`
    pub path: PathBuf,
    pub title: String,
    pub duration_s: Option<f64>,
    pub tool: ToolKind,
}

/// One way of getting audio from a URL.
pub trait Fetcher: Send + Sync {
    fn kind(&self) -> ToolKind;
    fn fetch(
        &self,
        ctx: &FetchCtx,
        sink: &mut dyn FnMut(DownloadEvent),
        cancel: &AtomicBool,
    ) -> Result<FetchOut, DownloadError>;
}

/// Where the tools are (resolved once per download from the settings + well-known directories).
#[derive(Debug, Clone, Default)]
pub struct Tools {
    pub bundled: Option<PathBuf>,
    pub system: Option<PathBuf>,
    pub deno: Option<PathBuf>,
    pub curl: Option<PathBuf>,
}

impl Tools {
    pub fn discover(system_override: Option<&str>) -> Self {
        let dirs = discover::default_tool_dirs(discover::home_dir().as_deref());
        Self::discover_in(system_override, &dirs, discover::bundled_ytdlp())
    }

    pub fn discover_in(
        system_override: Option<&str>,
        dirs: &[PathBuf],
        bundled: Option<PathBuf>,
    ) -> Self {
        let mut system = discover::find_system_ytdlp(system_override, dirs);
        // the same file twice is not a fallback
        if let (Some(a), Some(b)) = (&bundled, &system) {
            if a.canonicalize().ok() == b.canonicalize().ok() {
                system = None;
            }
        }
        Self {
            bundled,
            system,
            deno: discover::find_deno(dirs),
            curl: discover::find_curl(dirs),
        }
    }

    /// The attempt order: bundled yt-dlp → system yt-dlp → curl (direct media links only).
    pub fn fetchers(&self) -> Vec<Box<dyn Fetcher>> {
        let mut v: Vec<Box<dyn Fetcher>> = Vec::new();
        if let Some(p) = &self.bundled {
            v.push(Box::new(ytdlp::YtDlpFetcher {
                kind: ToolKind::Bundled,
                path: p.clone(),
                deno: self.deno.clone(),
            }));
        }
        if let Some(p) = &self.system {
            v.push(Box::new(ytdlp::YtDlpFetcher {
                kind: ToolKind::System,
                path: p.clone(),
                deno: self.deno.clone(),
            }));
        }
        if let Some(p) = &self.curl {
            v.push(Box::new(curl::CurlFetcher { path: p.clone() }));
        }
        v
    }
}

#[derive(Debug, Clone)]
pub struct Downloaded {
    /// `<job_dir>/audio.<ext>`
    pub path: PathBuf,
    pub title: String,
    pub duration_s: Option<f64>,
    pub bytes: u64,
    pub tool: ToolKind,
    pub url: String,
}

pub struct DownloadRequest {
    pub url: String,
    /// per-job directory (`<data>/downloads/<id>`); gets `audio.<ext>` + `meta.json` on success
    pub job_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub opts: DownloadOptions,
}

/// Run the chain of downloaders. On success the audio is at `<job_dir>/audio.<ext>`; on any failure nothing but the
/// (empty) job dir is left behind — a half-written file never reaches the transcription pipeline.
pub fn download(
    req: &DownloadRequest,
    fetchers: &[Box<dyn Fetcher>],
    sink: &mut dyn FnMut(DownloadEvent),
    cancel: &AtomicBool,
) -> Result<Downloaded, DownloadError> {
    let url = url::validate_url(&req.url)?;
    std::fs::create_dir_all(&req.job_dir).map_err(io_err(&req.job_dir))?;
    let stage = req.job_dir.join(".dl");
    let ctx = FetchCtx {
        url: url.clone(),
        stage_dir: stage.clone(),
        tmp_dir: req.cache_dir.join("tmp"),
        cache_dir: req.cache_dir.join("yt-dlp"),
        opts: req.opts.clone(),
    };
    let result = run_chain(&ctx, fetchers, sink, cancel);
    let out = match result {
        Ok(o) => o,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&stage);
            return Err(e);
        }
    };
    // publish atomically: the pipeline only ever sees the finished file under its final name
    let ext = out
        .path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_else(|| "audio".into());
    let dest = req.job_dir.join(format!("audio.{ext}"));
    let moved = std::fs::rename(&out.path, &dest);
    let _ = std::fs::remove_dir_all(&stage);
    moved.map_err(io_err(&dest))?;
    let bytes = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
    let d = Downloaded {
        path: dest,
        title: out.title,
        duration_s: out.duration_s,
        bytes,
        tool: out.tool,
        url,
    };
    store::write_meta(&req.job_dir, &d);
    Ok(d)
}

fn io_err(p: &Path) -> impl Fn(std::io::Error) -> DownloadError + '_ {
    move |e| {
        if e.raw_os_error() == Some(libc::ENOSPC) {
            return ytdlp::classify("No space left on device");
        }
        DownloadError::new(
            DownloadErrorKind::Extractor,
            "Couldn’t save the downloaded audio.",
            format!("{}: {e}", p.display()),
        )
    }
}

fn run_chain(
    ctx: &FetchCtx,
    fetchers: &[Box<dyn Fetcher>],
    sink: &mut dyn FnMut(DownloadEvent),
    cancel: &AtomicBool,
) -> Result<FetchOut, DownloadError> {
    if fetchers.is_empty() {
        return Err(DownloadError::no_downloader(
            "No yt-dlp (bundled or installed) and no curl were found.",
        ));
    }
    let mut failures: Vec<(ToolKind, DownloadError)> = Vec::new();
    let mut prev_real_failure: Option<(ToolKind, String)> = None;
    for f in fetchers {
        if cancel.load(Ordering::SeqCst) {
            return Err(DownloadError::cancelled());
        }
        let _ = std::fs::remove_dir_all(&ctx.stage_dir);
        std::fs::create_dir_all(&ctx.stage_dir).map_err(io_err(&ctx.stage_dir))?;
        if let Some((from, reason)) = prev_real_failure.take() {
            // say so in the UI — but don't announce curl for a link that is plainly not a file
            let worth_announcing =
                f.kind() != ToolKind::Curl || url::looks_like_audio_url(&ctx.url);
            if worth_announcing {
                sink(DownloadEvent::Fallback {
                    from,
                    to: f.kind(),
                    reason,
                });
            }
        }
        match f.fetch(ctx, sink, cancel) {
            Ok(out) => return Ok(out),
            Err(e) if !e.kind.is_retryable() => return Err(e),
            Err(e) => {
                tracing::warn!(
                    "{} failed ({:?}): {}",
                    f.kind().label(),
                    e.kind,
                    e.detail.lines().last().unwrap_or("")
                );
                if e.kind == DownloadErrorKind::Extractor {
                    prev_real_failure = Some((f.kind(), e.message.clone()));
                }
                failures.push((f.kind(), e));
            }
        }
    }
    let detail = failures
        .iter()
        .map(|(k, e)| format!("[{}] {}\n{}", k.id(), e.message, e.detail))
        .collect::<Vec<_>>()
        .join("\n\n");
    Err(DownloadError::no_downloader(detail))
}

/// `media.<ext>` in a staging directory, ignoring yt-dlp's in-progress `.part`/`.ytdl` files.
pub(crate) fn find_staged_media(stage: &Path) -> Option<PathBuf> {
    let mut best: Option<(u64, PathBuf)> = None;
    for e in std::fs::read_dir(stage).ok()?.flatten() {
        let p = e.path();
        let name = p.file_name()?.to_string_lossy().to_string();
        if !name.starts_with("media.") {
            continue;
        }
        let ext = name.rsplit('.').next().unwrap_or("");
        if matches!(
            ext,
            "part" | "ytdl" | "temp" | "tmp" | "json" | "vtt" | "srt" | "jpg" | "webp" | "png"
        ) {
            continue;
        }
        let len = e.metadata().map(|m| m.len()).unwrap_or(0);
        if len > 0 && best.as_ref().is_none_or(|(l, _)| len > *l) {
            best = Some((len, p));
        }
    }
    best.map(|(_, p)| p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A scripted downloader: what it returns, and a log of the calls.
    struct Fake {
        kind: ToolKind,
        result: Mutex<Option<Result<(), DownloadError>>>,
        calls: std::sync::Arc<Mutex<Vec<ToolKind>>>,
    }
    impl Fetcher for Fake {
        fn kind(&self) -> ToolKind {
            self.kind
        }
        fn fetch(
            &self,
            ctx: &FetchCtx,
            sink: &mut dyn FnMut(DownloadEvent),
            _c: &AtomicBool,
        ) -> Result<FetchOut, DownloadError> {
            self.calls.lock().unwrap().push(self.kind);
            match self.result.lock().unwrap().take().expect("called once") {
                Ok(()) => {
                    sink(DownloadEvent::Info {
                        title: format!("by {:?}", self.kind),
                        duration_s: Some(1.0),
                        playlist: false,
                    });
                    let p = ctx.stage_dir.join("media.m4a");
                    std::fs::write(&p, b"audio-bytes").unwrap();
                    // a stray partial file must never be picked up or kept
                    std::fs::write(ctx.stage_dir.join("media.m4a.part"), b"x").unwrap();
                    Ok(FetchOut {
                        path: p,
                        title: format!("by {:?}", self.kind),
                        duration_s: Some(1.0),
                        tool: self.kind,
                    })
                }
                Err(e) => Err(e),
            }
        }
    }

    type Calls = std::sync::Arc<Mutex<Vec<ToolKind>>>;
    fn chain(spec: Vec<(ToolKind, Result<(), DownloadError>)>) -> (Vec<Box<dyn Fetcher>>, Calls) {
        let calls: Calls = Default::default();
        let v = spec
            .into_iter()
            .map(|(kind, result)| {
                Box::new(Fake {
                    kind,
                    result: Mutex::new(Some(result)),
                    calls: calls.clone(),
                }) as Box<dyn Fetcher>
            })
            .collect();
        (v, calls)
    }
    fn extractor() -> DownloadError {
        DownloadError::new(
            DownloadErrorKind::Extractor,
            "The downloader couldn’t handle this link.",
            "ERROR: Unable to extract",
        )
    }
    fn not_applicable() -> DownloadError {
        DownloadError::new(
            DownloadErrorKind::NotApplicable,
            "This link is a web page, not an audio file.",
            "text/html",
        )
    }
    fn req(dir: &Path, url: &str) -> DownloadRequest {
        DownloadRequest {
            url: url.into(),
            job_dir: dir.join("job"),
            cache_dir: dir.join("cache"),
            opts: DownloadOptions::default(),
        }
    }
    fn run(
        dir: &Path,
        url: &str,
        fetchers: &[Box<dyn Fetcher>],
    ) -> (Result<Downloaded, DownloadError>, Vec<DownloadEvent>) {
        let mut ev = vec![];
        let r = download(
            &req(dir, url),
            fetchers,
            &mut |e| ev.push(e),
            &AtomicBool::new(false),
        );
        (r, ev)
    }

    #[test]
    fn first_downloader_wins_and_the_file_is_published_atomically() {
        let t = tempfile::tempdir().unwrap();
        let (f, calls) = chain(vec![
            (ToolKind::Bundled, Ok(())),
            (ToolKind::System, Ok(())),
        ]);
        let (r, ev) = run(t.path(), "https://a.com/v", &f);
        let d = r.unwrap();
        assert_eq!(*calls.lock().unwrap(), vec![ToolKind::Bundled]);
        assert_eq!(d.path, t.path().join("job/audio.m4a"));
        assert_eq!(std::fs::read(&d.path).unwrap(), b"audio-bytes");
        assert!(
            !t.path().join("job/.dl").exists(),
            "staging (and its .part file) is gone"
        );
        assert!(ev
            .iter()
            .all(|e| !matches!(e, DownloadEvent::Fallback { .. })));
        let meta = store::read_meta(&t.path().join("job")).unwrap();
        assert_eq!(
            (meta.title.as_str(), meta.url.as_str()),
            ("by Bundled", "https://a.com/v")
        );
    }

    #[test]
    fn bundled_failure_retries_once_with_system_and_says_so() {
        let t = tempfile::tempdir().unwrap();
        let (f, calls) = chain(vec![
            (ToolKind::Bundled, Err(extractor())),
            (ToolKind::System, Ok(())),
            (ToolKind::Curl, Ok(())),
        ]);
        let (r, ev) = run(t.path(), "https://www.youtube.com/watch?v=1", &f);
        assert_eq!(r.unwrap().tool, ToolKind::System);
        assert_eq!(
            *calls.lock().unwrap(),
            vec![ToolKind::Bundled, ToolKind::System]
        );
        assert!(ev.contains(&DownloadEvent::Fallback {
            from: ToolKind::Bundled,
            to: ToolKind::System,
            reason: "The downloader couldn’t handle this link.".into()
        }));
    }

    #[test]
    fn curl_is_the_last_resort_for_direct_audio_links_and_announced_only_then() {
        let t = tempfile::tempdir().unwrap();
        let (f, calls) = chain(vec![
            (ToolKind::Bundled, Err(extractor())),
            (ToolKind::Curl, Ok(())),
        ]);
        let (r, ev) = run(t.path(), "https://cdn.example.com/pod/ep12.mp3", &f);
        assert_eq!(r.unwrap().tool, ToolKind::Curl);
        assert_eq!(
            *calls.lock().unwrap(),
            vec![ToolKind::Bundled, ToolKind::Curl]
        );
        assert!(ev.iter().any(|e| matches!(
            e,
            DownloadEvent::Fallback {
                to: ToolKind::Curl,
                ..
            }
        )));
        // a web-page URL: curl is still asked (it checks Content-Type), but nothing is announced
        let t2 = tempfile::tempdir().unwrap();
        let (f, _) = chain(vec![
            (ToolKind::Bundled, Err(extractor())),
            (ToolKind::Curl, Err(not_applicable())),
        ]);
        let (r, ev) = run(t2.path(), "https://example.com/some/page", &f);
        assert_eq!(r.unwrap_err().kind, DownloadErrorKind::NoDownloader);
        assert!(ev
            .iter()
            .all(|e| !matches!(e, DownloadEvent::Fallback { .. })));
    }

    #[test]
    fn nothing_works_gives_the_actionable_message_with_every_tools_output() {
        let t = tempfile::tempdir().unwrap();
        let (f, _) = chain(vec![
            (ToolKind::Bundled, Err(extractor())),
            (ToolKind::System, Err(extractor())),
            (ToolKind::Curl, Err(not_applicable())),
        ]);
        let (r, _) = run(t.path(), "https://example.com/x", &f);
        let e = r.unwrap_err();
        assert_eq!(e.kind, DownloadErrorKind::NoDownloader);
        assert_eq!(e.message, "Couldn’t download this link. Install or update yt-dlp: `brew install yt-dlp` / `brew upgrade yt-dlp`.");
        for tool in ["[bundled]", "[system]", "[curl]"] {
            assert!(e.detail.contains(tool), "{tool} missing from {}", e.detail);
        }
        assert!(!t.path().join("job/.dl").exists());
        assert!(!t.path().join("job/audio.m4a").exists());
    }

    #[test]
    fn verdicts_about_the_link_do_not_fall_back() {
        for kind in [
            DownloadErrorKind::Live,
            DownloadErrorKind::NeedsLogin,
            DownloadErrorKind::Drm,
            DownloadErrorKind::NoAudio,
            DownloadErrorKind::NotFound,
            DownloadErrorKind::Network,
            DownloadErrorKind::DiskFull,
        ] {
            let t = tempfile::tempdir().unwrap();
            let (f, calls) = chain(vec![
                (
                    ToolKind::Bundled,
                    Err(DownloadError::new(kind, "verdict", "d")),
                ),
                (ToolKind::System, Ok(())),
            ]);
            let (r, _) = run(t.path(), "https://a.com/v", &f);
            assert_eq!(r.unwrap_err().kind, kind);
            assert_eq!(
                *calls.lock().unwrap(),
                vec![ToolKind::Bundled],
                "{kind:?} must not try the next tool"
            );
        }
    }

    #[test]
    fn no_tools_at_all() {
        let t = tempfile::tempdir().unwrap();
        let (r, _) = run(t.path(), "https://a.com/v", &[]);
        assert_eq!(r.unwrap_err().kind, DownloadErrorKind::NoDownloader);
    }

    #[test]
    fn bad_urls_never_reach_a_tool() {
        let t = tempfile::tempdir().unwrap();
        let (f, calls) = chain(vec![(ToolKind::Bundled, Ok(()))]);
        for bad in ["", "ftp://a.com/x", "not a url", "file:///etc/passwd"] {
            let (r, _) = run(t.path(), bad, &f);
            assert_eq!(r.unwrap_err().kind, DownloadErrorKind::InvalidUrl, "{bad}");
        }
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn cancel_before_start_runs_nothing() {
        let t = tempfile::tempdir().unwrap();
        let (f, calls) = chain(vec![(ToolKind::Bundled, Ok(()))]);
        let cancel = AtomicBool::new(true);
        let r = download(&req(t.path(), "https://a.com/v"), &f, &mut |_| {}, &cancel);
        assert_eq!(r.unwrap_err().kind, DownloadErrorKind::Cancelled);
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn staged_media_ignores_partials() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("media.webm.part"), vec![0u8; 100]).unwrap();
        std::fs::write(t.path().join("media.webm.ytdl"), b"x").unwrap();
        assert_eq!(find_staged_media(t.path()), None);
        std::fs::write(t.path().join("media.webm"), b"abc").unwrap();
        assert_eq!(
            find_staged_media(t.path()),
            Some(t.path().join("media.webm"))
        );
        std::fs::write(t.path().join("media.empty"), b"").unwrap();
        assert_eq!(
            find_staged_media(t.path()),
            Some(t.path().join("media.webm"))
        );
    }

    #[test]
    fn tools_order_and_dedup() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let bin = t.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        for n in ["yt-dlp", "deno"] {
            let p = bin.join(n);
            std::fs::write(&p, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let bundled = t.path().join("app/yt-dlp");
        std::fs::create_dir_all(bundled.parent().unwrap()).unwrap();
        std::fs::write(&bundled, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&bundled, std::fs::Permissions::from_mode(0o755)).unwrap();
        let tools = Tools::discover_in(None, &[bin.clone()], Some(bundled.clone()));
        assert_eq!(tools.bundled.as_ref(), Some(&bundled));
        assert_eq!(tools.system, Some(bin.join("yt-dlp")));
        assert_eq!(tools.deno, Some(bin.join("deno")));
        let kinds: Vec<_> = tools.fetchers().iter().map(|f| f.kind()).collect();
        assert_eq!(&kinds[..2], &[ToolKind::Bundled, ToolKind::System]);
        // pointing the system override at the bundled file is not a second downloader
        let same = Tools::discover_in(
            Some(bundled.to_str().unwrap()),
            &[bin],
            Some(bundled.clone()),
        );
        assert_eq!(same.system, None);
    }
}
