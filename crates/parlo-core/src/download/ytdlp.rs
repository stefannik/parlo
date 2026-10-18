//! yt-dlp as a child process: capability probing, flag construction, progress/`-J` parsing, error classification.
//! Everything except [`YtDlpFetcher::fetch`] is pure and unit-tested without a network (DECISIONS C-021/C-022).

use super::error::{DownloadError, DownloadErrorKind as K};
use super::proc::{self, Live, RunError};
use super::{DownloadEvent, DownloadOptions, FetchCtx, FetchOut, Fetcher, ToolKind};
use serde_json::Value;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

/// The only audio selection Parlo asks for: m4a if offered (AAC, decodable everywhere), else the best audio stream
/// (webm/opus). Never `-x`: no post-processing, so no ffmpeg is needed.
pub const FORMAT: &str = "bestaudio[ext=m4a]/bestaudio";
/// Prefix of every machine-readable progress line (`--progress-template`).
pub const PROGRESS_PREFIX: &str = "PARLO_DL";
/// First yt-dlp release that has `--js-runtimes` (the external JS runtime option; DECISIONS C-022).
pub const JS_RUNTIMES_SINCE: (u32, u32, u32) = (2025, 11, 12);
pub const SLEEP_MIN: u32 = 5;
pub const SLEEP_MAX: u32 = 15;
const PROBE_TIMEOUT: Duration = Duration::from_secs(150);

// ---------------------------------------------------------------------------------------------------- capabilities

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caps {
    pub version: String,
    /// `--list-impersonate-targets` shows an available Chrome target (None = not asked)
    pub impersonate_chrome: Option<bool>,
}

impl Caps {
    pub fn supports_js_runtimes(&self) -> bool {
        parse_version(&self.version).is_some_and(|v| v >= JS_RUNTIMES_SINCE)
    }
}

/// `2026.08.19` / `2025.11.12.232843` / `2026.8.19` → (2026, 8, 19)
pub fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let mut it = s.trim().split('.');
    let y = it.next()?.parse().ok()?;
    let m = it.next()?.parse().ok()?;
    let d = it.next()?.parse().ok()?;
    Some((y, m, d))
}

/// `--list-impersonate-targets` prints a table; a row is usable unless it says `(unavailable)`:
/// ```text
/// Client          OS           Source
/// Chrome-133      Macos-15     curl_cffi
/// Chrome          -            curl_cffi (unavailable)
/// ```
pub fn parse_impersonate_chrome(out: &str) -> bool {
    out.lines().any(|l| {
        let t = l.trim_start().to_ascii_lowercase();
        t.starts_with("chrome") && !t.contains("unavailable")
    })
}

type CapKey = (PathBuf, u64, Option<SystemTime>);
fn cap_cache() -> &'static Mutex<HashMap<CapKey, Caps>> {
    static C: std::sync::OnceLock<Mutex<HashMap<CapKey, Caps>>> = std::sync::OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Version (+ impersonation targets when `want_impersonate`) of a yt-dlp binary; cached per file (path, size, mtime) for
/// the life of the process, since the onefile build needs ~1 s per launch.
pub fn capabilities(
    path: &Path,
    want_impersonate: bool,
    cancel: &AtomicBool,
) -> Result<Caps, DownloadError> {
    let md = std::fs::metadata(path).ok();
    let key = (
        path.to_path_buf(),
        md.as_ref().map(|m| m.len()).unwrap_or(0),
        md.and_then(|m| m.modified().ok()),
    );
    let cached = cap_cache().lock().unwrap().get(&key).cloned();
    let need_version = cached.is_none();
    let need_imp = want_impersonate
        && cached
            .as_ref()
            .is_none_or(|c| c.impersonate_chrome.is_none());
    let version_cmd = || {
        proc::run_capture(
            proc::command(path, &[OsString::from("--version")], None),
            cancel,
            Duration::from_secs(30),
        )
    };
    let imp_cmd = || {
        let args = ["--no-config", "--list-impersonate-targets"].map(OsString::from);
        proc::run_capture(
            proc::command(path, &args, None),
            cancel,
            Duration::from_secs(30),
        )
    };
    // each launch of a PyInstaller build costs 0.5–1 s: ask both questions at the same time
    let (version_res, imp_res) = std::thread::scope(|sc| {
        let imp = need_imp.then(|| sc.spawn(imp_cmd));
        let ver = need_version.then(version_cmd);
        (
            ver,
            imp.map(|h| h.join().expect("impersonation probe panicked")),
        )
    });
    let mut caps = match (cached, version_res) {
        (Some(c), _) => c,
        (None, Some(res)) => {
            let (r, out) = res.map_err(|e| run_err(e, path))?;
            if !r.status.success() {
                return Err(DownloadError::new(
                    K::Extractor,
                    "The downloader didn’t start.",
                    format!("{} --version: {}", path.display(), r.stderr_text()),
                ));
            }
            Caps {
                version: out.trim().to_string(),
                impersonate_chrome: None,
            }
        }
        (None, None) => unreachable!("no cache entry means the version was asked for"),
    };
    if let Some(res) = imp_res {
        let (r, out) = res.map_err(|e| run_err(e, path))?;
        // an old yt-dlp without the option exits non-zero: treat as "no impersonation"
        caps.impersonate_chrome = Some(r.status.success() && parse_impersonate_chrome(&out));
    }
    cap_cache().lock().unwrap().insert(key, caps.clone());
    Ok(caps)
}

fn run_err(e: RunError, path: &Path) -> DownloadError {
    match e {
        RunError::Cancelled => DownloadError::cancelled(),
        RunError::TimedOut => DownloadError::new(
            K::Extractor,
            "The downloader took too long to answer.",
            format!("{}: timed out", path.display()),
        ),
        RunError::Spawn(io) => DownloadError::new(
            K::Extractor,
            "The downloader couldn’t be started.",
            format!("{}: {io}", path.display()),
        ),
    }
}

// ---------------------------------------------------------------------------------------------------- flags

pub struct FlagCtx<'a> {
    pub opts: &'a DownloadOptions,
    pub caps: &'a Caps,
    pub deno: Option<&'a Path>,
    pub cache_dir: &'a Path,
}

fn os<S: Into<OsString>>(s: S) -> OsString {
    s.into()
}

/// Flags shared by the probe and the download. Order is stable (tests assert on it).
pub fn common_args(c: &FlagCtx) -> Vec<OsString> {
    let mut a: Vec<OsString> = vec![
        os("--no-config"), // never read the user's global yt-dlp config: behaviour must be predictable
        os("--no-playlist"),
        os("--playlist-items"),
        os("1"),
        os("--no-warnings"),
        os("--no-mtime"),
        os("--cache-dir"),
        c.cache_dir.into(),
        os("--socket-timeout"),
        os("30"),
    ];
    if c.opts.impersonate {
        if c.caps.impersonate_chrome == Some(true) {
            a.extend([os("--impersonate"), os("chrome")]);
        } else {
            tracing::warn!(
                "browser impersonation requested but this yt-dlp ({}) has no usable Chrome target — continuing without it",
                c.caps.version
            );
        }
    }
    if let Some(b) = &c.opts.cookies_browser {
        if super::discover::is_known_browser(b) {
            a.extend([os("--cookies-from-browser"), os(b.as_str())]);
        }
    }
    if let Some(d) = c.deno {
        if c.caps.supports_js_runtimes() {
            let mut v = OsString::from("deno:");
            v.push(d);
            a.extend([os("--js-runtimes"), v]);
        }
    }
    a
}

/// `-J`: one JSON document on stdout, nothing downloaded.
pub fn probe_args(c: &FlagCtx, url: &str) -> Vec<OsString> {
    let mut a = common_args(c);
    a.extend([os("-J"), os("--"), os(url)]);
    a
}

/// The download itself. `stage` is the directory the file lands in as `media.<ext>` (yt-dlp keeps `.part` until done).
pub fn download_args(c: &FlagCtx, url: &str, stage: &Path) -> Vec<OsString> {
    let mut a = common_args(c);
    a.extend([os("-f"), os(FORMAT), os("--newline")]);
    a.extend([
        os("--progress-template"),
        os(format!(
            "download:{PROGRESS_PREFIX} %(progress.status)s|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.total_bytes_estimate)s|%(progress.speed)s|%(progress.eta)s"
        )),
    ]);
    if c.opts.rate_limit_sleep {
        // yt-dlp sleeps before EACH download, including the first — the UI shows "Waiting N s" for it
        a.extend([
            os("--sleep-interval"),
            os(SLEEP_MIN.to_string()),
            os("--max-sleep-interval"),
            os(SLEEP_MAX.to_string()),
        ]);
    }
    let out = stage.join("media.%(ext)s").into_os_string();
    a.extend([os("-o"), out, os("--"), os(url)]);
    a
}

// ---------------------------------------------------------------------------------------------------- parsing

#[derive(Debug, Clone, PartialEq)]
pub enum Line {
    Progress {
        downloaded: u64,
        total: Option<u64>,
        speed_bps: Option<f64>,
        eta_s: Option<f64>,
    },
    /// `[download] Sleeping 7.00 seconds ...`
    Sleeping(f64),
    Other,
}

fn num(s: &str) -> Option<f64> {
    let v: f64 = s.trim().parse().ok()?;
    (v.is_finite() && v >= 0.0).then_some(v)
}

pub fn parse_line(l: &str) -> Line {
    if let Some(rest) = l.strip_prefix(PROGRESS_PREFIX) {
        let f: Vec<&str> = rest.trim().split('|').collect();
        // status|downloaded|total|estimate|speed|eta
        if f.len() >= 6 {
            let status = f[0].trim_start_matches(' ');
            let downloaded = num(f[1]).unwrap_or(0.0) as u64;
            let total = num(f[2])
                .or_else(|| num(f[3]))
                .map(|t| t as u64)
                .filter(|t| *t > 0);
            let _ = status;
            return Line::Progress {
                downloaded,
                total,
                speed_bps: num(f[4]),
                eta_s: num(f[5]),
            };
        }
    }
    if let Some(rest) = l.strip_prefix("[download] Sleeping ") {
        if let Some(n) = rest
            .split_whitespace()
            .next()
            .and_then(|n| n.parse::<f64>().ok())
        {
            return Line::Sleeping(n);
        }
    }
    Line::Other
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Probe {
    pub title: Option<String>,
    pub uploader: Option<String>,
    pub duration_s: Option<f64>,
    pub live: bool,
    pub has_audio: bool,
    pub all_drm: bool,
    pub filesize: Option<u64>,
    /// the link was a playlist; only its first item is used
    pub playlist: bool,
}

pub fn parse_probe(json: &str) -> Result<Probe, String> {
    let v: Value =
        serde_json::from_str(json.trim()).map_err(|e| format!("unreadable probe output: {e}"))?;
    let (info, playlist) = match v.get("_type").and_then(Value::as_str) {
        Some("playlist") => {
            let first = v
                .get("entries")
                .and_then(Value::as_array)
                .and_then(|e| e.iter().find(|x| !x.is_null()))
                .cloned()
                .ok_or_else(|| "the playlist is empty".to_string())?;
            (first, true)
        }
        _ => (v, false),
    };
    let s = |k: &str| {
        info.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|x| !x.is_empty())
            .map(String::from)
    };
    let status = info
        .get("live_status")
        .and_then(Value::as_str)
        .unwrap_or("");
    let live = info
        .get("is_live")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || matches!(status, "is_live" | "is_upcoming" | "post_live");
    let formats = info.get("formats").and_then(Value::as_array);
    let has_audio = match formats {
        Some(f) if !f.is_empty() => f
            .iter()
            .any(|x| x.get("acodec").and_then(Value::as_str) != Some("none")),
        _ => true, // direct links list no formats: assume there is audio, the decoder is the judge
    };
    let all_drm = formats.is_some_and(|f| {
        !f.is_empty()
            && f.iter()
                .all(|x| x.get("has_drm").and_then(Value::as_bool) == Some(true))
    });
    Ok(Probe {
        title: s("title"),
        uploader: s("uploader").or_else(|| s("channel")),
        duration_s: info
            .get("duration")
            .and_then(Value::as_f64)
            .filter(|d| *d > 0.0),
        live,
        has_audio,
        all_drm,
        filesize: info
            .get("filesize")
            .or_else(|| info.get("filesize_approx"))
            .and_then(Value::as_f64)
            .map(|f| f as u64),
        playlist,
    })
}

/// Map yt-dlp's stderr to something the user can act on. Order matters (most specific first).
pub fn classify(stderr: &str) -> DownloadError {
    let low = stderr.to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| low.contains(n));
    let (kind, msg): (K, &str) = if has(&["no space left", "errno 28", "disk quota"]) {
        (
            K::DiskFull,
            "Not enough free disk space to download this audio. Free some space and try again.",
        )
    } else if has(&[
        "drm protect",
        "drm-protect",
        "known to use drm",
        "[drm]",
        "widevine",
        "fairplay",
    ]) {
        (K::Drm, "This content is protected (DRM) or comes from a site that doesn’t allow downloads (like Spotify or Netflix), so Parlo can’t read it.")
    } else if has(&[
        "cookies database",
        "binarycookies",
        "failed to decrypt",
        "keychain",
        "keyring",
        "cookies from browser",
        "could not find firefox",
        "unsupported browser",
        "cookie database",
    ]) || (low.contains("cookie")
        && has(&[
            "operation not permitted",
            "permission denied",
            "errno 1]",
            "errno 13]",
        ]))
    {
        (
            K::Cookies,
            "Couldn’t read your browser’s cookies. In Settings → Downloads, check the browser under “Use browser cookies” (close it and try again, allow the Keychain prompt, or give Parlo Full Disk Access for Safari) — or turn the option off.",
        )
    } else if has(&[
        "private video",
        "this video is private",
        "members-only",
        "members only",
        "join this channel",
        "login required",
        "log in",
        "sign in",
        "age-restricted",
        "age restricted",
        "confirm your age",
        "age verification",
        "only available for registered users",
        "available to premium",
        "requires authentication",
        "cookies",
        "not a bot",
    ]) {
        (K::NeedsLogin, "This video is private, members-only or age-restricted. If your browser is signed in to the site, turn on Settings → Downloads → “Use browser cookies” and try again.")
    } else if has(&[
        "live event will begin",
        "premieres in",
        "this live stream",
        "is a live stream",
        "live stream recording",
    ]) {
        (K::Live, "This is a live stream, so there is nothing to transcribe yet. Try again after it has ended.")
    } else if has(&[
        "http error 404",
        "http error 410",
        "video unavailable",
        "this video is not available",
        "has been removed",
        "does not exist",
        "no longer available",
    ]) {
        (K::NotFound, "That link doesn’t seem to exist any more (not found). Check the address and try again.")
    } else if has(&[
        "requested format is not available",
        "no video formats found",
        "no audio",
    ]) {
        (K::NoAudio, "Couldn’t find an audio track at this link.")
    } else if has(&[
        "name or service not known",
        "temporary failure in name resolution",
        "nodename nor servname",
        "getaddrinfo failed",
        "network is unreachable",
        "connection refused",
        "timed out",
        "no route to host",
        "failed to establish a new connection",
        "unable to connect to proxy",
        "connection reset",
        "internet connection appears to be offline",
        "remote end closed connection",
        "could not resolve host",
        // curl_cffi (impersonation) words its transport errors differently
        "failed to connect",
        "could not connect to server",
        "couldn't resolve host",
        "curl: (6)",
        "curl: (7)",
        "curl: (28)",
        "operation timed out",
    ]) {
        (
            K::Network,
            "Couldn’t reach the website. Check your internet connection and try again.",
        )
    } else {
        (K::Extractor, "The downloader couldn’t handle this link.")
    };
    DownloadError::new(kind, msg, tail(stderr, 2000))
}

fn tail(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.len() <= max {
        return s.to_string();
    }
    let mut start = s.len() - max;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", &s[start..])
}

// ---------------------------------------------------------------------------------------------------- the fetcher

pub struct YtDlpFetcher {
    pub kind: ToolKind,
    pub path: PathBuf,
    pub deno: Option<PathBuf>,
}

impl YtDlpFetcher {
    fn probe(
        &self,
        ctx: &FetchCtx,
        flags: &FlagCtx,
        cancel: &AtomicBool,
    ) -> Result<Probe, DownloadError> {
        let args = probe_args(flags, &ctx.url);
        let (r, out) = proc::run_capture(
            proc::command(&self.path, &args, Some(&ctx.tmp_dir)),
            cancel,
            PROBE_TIMEOUT,
        )
        .map_err(|e| run_err(e, &self.path))?;
        if !r.status.success() {
            return Err(classify(&r.stderr_text()));
        }
        parse_probe(&out).map_err(|m| {
            DownloadError::new(K::Extractor, "The downloader couldn’t handle this link.", m)
        })
    }
}

impl Fetcher for YtDlpFetcher {
    fn kind(&self) -> ToolKind {
        self.kind
    }

    fn fetch(
        &self,
        ctx: &FetchCtx,
        sink: &mut dyn FnMut(DownloadEvent),
        cancel: &AtomicBool,
    ) -> Result<FetchOut, DownloadError> {
        let caps = capabilities(&self.path, ctx.opts.impersonate, cancel)?;
        let flags = FlagCtx {
            opts: &ctx.opts,
            caps: &caps,
            deno: self.deno.as_deref(),
            cache_dir: &ctx.cache_dir,
        };
        let probe = self.probe(ctx, &flags, cancel)?;
        if probe.live {
            return Err(DownloadError::new(
                K::Live,
                classify("this live stream").message,
                "probe: is_live / is_upcoming",
            ));
        }
        if probe.all_drm {
            return Err(classify("[drm]"));
        }
        if !probe.has_audio {
            return Err(DownloadError::new(
                K::NoAudio,
                "Couldn’t find an audio track at this link.",
                "probe: no format has an audio codec",
            ));
        }
        if let Some(need) = probe.filesize {
            let free = crate::paths::free_disk_bytes(&ctx.stage_dir).unwrap_or(u64::MAX);
            if free < need.saturating_add(100 << 20) {
                return Err(classify("No space left on device"));
            }
        }
        let title = probe
            .title
            .clone()
            .unwrap_or_else(|| super::url::fallback_title(&ctx.url));
        sink(DownloadEvent::Info {
            title: title.clone(),
            duration_s: probe.duration_s,
            playlist: probe.playlist,
        });

        let args = download_args(&flags, &ctx.url, &ctx.stage_dir);
        let mut last_progress_sent = std::time::Instant::now() - Duration::from_secs(1);
        // the newest update the throttle held back — flushed at the end so the bar reaches 100 %
        let mut held: Option<DownloadEvent> = None;
        let r = proc::run_streaming(
            proc::command(&self.path, &args, Some(&ctx.tmp_dir)),
            Live::Stdout,
            cancel,
            None,
            &mut |l| match parse_line(l) {
                Line::Sleeping(secs) => sink(DownloadEvent::Waiting { secs }),
                Line::Progress {
                    downloaded,
                    total,
                    speed_bps,
                    eta_s,
                } => {
                    let ev = DownloadEvent::Progress {
                        downloaded,
                        total,
                        speed_bps,
                        eta_s,
                    };
                    // ≤ 10 updates per second is plenty for a progress bar
                    if last_progress_sent.elapsed() >= Duration::from_millis(100) {
                        last_progress_sent = std::time::Instant::now();
                        held = None;
                        sink(ev);
                    } else {
                        held = Some(ev);
                    }
                }
                Line::Other => {}
            },
        )
        .map_err(|e| run_err(e, &self.path))?;
        if let Some(ev) = held {
            sink(ev);
        }
        if !r.status.success() {
            return Err(classify(&r.stderr_text()));
        }
        let path = super::find_staged_media(&ctx.stage_dir).ok_or_else(|| {
            DownloadError::new(
                K::Extractor,
                "The downloader finished but produced no audio file.",
                r.stderr_text(),
            )
        })?;
        Ok(FetchOut {
            path,
            title,
            duration_s: probe.duration_s,
            tool: self.kind,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> DownloadOptions {
        DownloadOptions {
            impersonate: true,
            cookies_browser: None,
            rate_limit_sleep: true,
        }
    }
    fn caps(v: &str, imp: Option<bool>) -> Caps {
        Caps {
            version: v.into(),
            impersonate_chrome: imp,
        }
    }
    fn strs(a: Vec<OsString>) -> Vec<String> {
        a.into_iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect()
    }
    fn has_pair(a: &[String], k: &str, v: &str) -> bool {
        a.windows(2).any(|w| w[0] == k && w[1] == v)
    }

    #[test]
    fn version_parsing_and_js_runtime_gate() {
        assert_eq!(parse_version("2026.08.19"), Some((2026, 8, 19)));
        assert_eq!(parse_version("2025.11.12.232843\n"), Some((2025, 11, 12)));
        assert_eq!(parse_version("nonsense"), None);
        assert!(caps("2026.08.19", None).supports_js_runtimes());
        assert!(caps("2025.11.12", None).supports_js_runtimes());
        assert!(!caps("2025.10.22", None).supports_js_runtimes());
        assert!(!caps("garbage", None).supports_js_runtimes());
    }

    #[test]
    fn impersonate_target_table() {
        let ok = "[info] Available impersonate targets\nClient          OS           Source\n-----\nChrome-133      Macos-15     curl_cffi\nSafari-18.0     Ios-18.0     curl_cffi\n";
        assert!(parse_impersonate_chrome(ok));
        let unavailable = "Client OS Source\n---\nChrome          -            curl_cffi (unavailable)\nEdge   -  curl_cffi (unavailable)\n";
        assert!(!parse_impersonate_chrome(unavailable));
        assert!(!parse_impersonate_chrome(
            "Safari-18.0 Ios curl_cffi\nFirefox-133 Macos curl_cffi"
        ));
        assert!(!parse_impersonate_chrome(""));
    }

    #[test]
    fn download_flags_are_exactly_what_the_brief_asks_for() {
        let c = caps("2026.08.19", Some(true));
        let o = opts();
        let deno = PathBuf::from("/opt/homebrew/bin/deno");
        let f = FlagCtx {
            opts: &o,
            caps: &c,
            deno: Some(&deno),
            cache_dir: Path::new("/cache/yt"),
        };
        let a = strs(download_args(
            &f,
            "https://youtu.be/x",
            Path::new("/data/downloads/j/.dl"),
        ));
        assert_eq!(a[0], "--no-config");
        for flag in ["--no-playlist", "--no-warnings", "--no-mtime", "--newline"] {
            assert!(a.iter().any(|x| x == flag), "missing {flag}");
        }
        assert!(has_pair(&a, "-f", "bestaudio[ext=m4a]/bestaudio"));
        assert!(has_pair(&a, "--impersonate", "chrome"));
        assert!(has_pair(&a, "--sleep-interval", "5"));
        assert!(has_pair(&a, "--max-sleep-interval", "15"));
        assert!(has_pair(&a, "--js-runtimes", "deno:/opt/homebrew/bin/deno"));
        assert!(has_pair(&a, "--cache-dir", "/cache/yt"));
        assert!(has_pair(&a, "-o", "/data/downloads/j/.dl/media.%(ext)s"));
        assert!(!a
            .iter()
            .any(|x| x == "-x" || x == "--extract-audio" || x == "--no-part"));
        assert!(a.iter().any(|x| x.starts_with("download:PARLO_DL ")));
        // the URL is last and comes after `--` so it can never be parsed as an option
        assert_eq!(&a[a.len() - 2..], ["--", "https://youtu.be/x"]);
    }

    #[test]
    fn optional_flags_follow_the_settings_and_capabilities() {
        let stage = Path::new("/s");
        let cache = Path::new("/c");
        // all toggles off
        let o = DownloadOptions {
            impersonate: false,
            cookies_browser: None,
            rate_limit_sleep: false,
        };
        let c = caps("2026.08.19", Some(true));
        let a = strs(download_args(
            &FlagCtx {
                opts: &o,
                caps: &c,
                deno: None,
                cache_dir: cache,
            },
            "https://a.com/x",
            stage,
        ));
        for flag in [
            "--impersonate",
            "--sleep-interval",
            "--cookies-from-browser",
            "--js-runtimes",
        ] {
            assert!(!a.iter().any(|x| x == flag), "{flag} should be absent");
        }
        // impersonation wanted but this binary has no chrome target → skipped (and logged)
        let o = opts();
        let c = caps("2026.08.19", Some(false));
        let a = strs(common_args(&FlagCtx {
            opts: &o,
            caps: &c,
            deno: None,
            cache_dir: cache,
        }));
        assert!(!a.iter().any(|x| x == "--impersonate"));
        // never queried (None) → also skipped
        let c = caps("2026.08.19", None);
        assert!(!strs(common_args(&FlagCtx {
            opts: &o,
            caps: &c,
            deno: None,
            cache_dir: cache
        }))
        .iter()
        .any(|x| x == "--impersonate"));
        // cookies: a known browser is passed, an unknown one is dropped
        let mut o = opts();
        o.cookies_browser = Some("firefox".into());
        let c = caps("2026.08.19", Some(true));
        let a = strs(common_args(&FlagCtx {
            opts: &o,
            caps: &c,
            deno: None,
            cache_dir: cache,
        }));
        assert!(has_pair(&a, "--cookies-from-browser", "firefox"));
        o.cookies_browser = Some("chrome --exec evil".into());
        assert!(!strs(common_args(&FlagCtx {
            opts: &o,
            caps: &c,
            deno: None,
            cache_dir: cache
        }))
        .iter()
        .any(|x| x == "--cookies-from-browser"));
        // deno is only passed to a yt-dlp that knows --js-runtimes
        let old = caps("2025.06.01", Some(true));
        let deno = PathBuf::from("/usr/local/bin/deno");
        let a = strs(common_args(&FlagCtx {
            opts: &opts(),
            caps: &old,
            deno: Some(&deno),
            cache_dir: cache,
        }));
        assert!(!a.iter().any(|x| x == "--js-runtimes"));
    }

    #[test]
    fn probe_flags_download_nothing() {
        let c = caps("2026.08.19", Some(true));
        let o = opts();
        let f = FlagCtx {
            opts: &o,
            caps: &c,
            deno: None,
            cache_dir: Path::new("/c"),
        };
        let a = strs(probe_args(&f, "https://a.com/v"));
        assert!(a.iter().any(|x| x == "-J"));
        assert!(!a
            .iter()
            .any(|x| x == "-f" || x == "--sleep-interval" || x == "-o"));
        assert_eq!(&a[a.len() - 2..], ["--", "https://a.com/v"]);
    }

    #[test]
    fn parses_progress_lines() {
        // exactly what yt-dlp 2026.08.19 printed against the local server
        assert_eq!(
            parse_line("PARLO_DL downloading|7168|113136|NA|71694.59769879216|1"),
            Line::Progress {
                downloaded: 7168,
                total: Some(113136),
                speed_bps: Some(71694.59769879216),
                eta_s: Some(1.0)
            }
        );
        assert_eq!(
            parse_line("PARLO_DL downloading|1024|NA|2048000.0|NA|NA"),
            Line::Progress {
                downloaded: 1024,
                total: Some(2_048_000),
                speed_bps: None,
                eta_s: None
            }
        );
        assert_eq!(
            parse_line("PARLO_DL finished|113136|113136|NA|62572.9|NA"),
            Line::Progress {
                downloaded: 113136,
                total: Some(113136),
                speed_bps: Some(62572.9),
                eta_s: None
            }
        );
        assert_eq!(
            parse_line("PARLO_DL downloading|NA|NA|NA|NA|NA"),
            Line::Progress {
                downloaded: 0,
                total: None,
                speed_bps: None,
                eta_s: None
            }
        );
        assert_eq!(
            parse_line("[download] Destination: x/media.mp3"),
            Line::Other
        );
        assert_eq!(parse_line("PARLO_DL garbage"), Line::Other);
    }

    #[test]
    fn parses_the_sleep_line() {
        assert_eq!(
            parse_line("[download] Sleeping 1.68 seconds ..."),
            Line::Sleeping(1.68)
        );
        assert_eq!(
            parse_line("[download] Sleeping 12.00 seconds ..."),
            Line::Sleeping(12.0)
        );
        assert_eq!(parse_line("[download] Sleeping soon"), Line::Other);
    }

    #[test]
    fn probe_json_variants() {
        let p = parse_probe(r#"{"title":" Local first ","duration":3725.5,"uploader":"A Show","formats":[{"acodec":"none","vcodec":"avc1"},{"acodec":"mp4a.40.2","vcodec":"none"}],"filesize_approx":1000}"#).unwrap();
        assert_eq!(p.title.as_deref(), Some("Local first"));
        assert_eq!(p.duration_s, Some(3725.5));
        assert!(p.has_audio && !p.live && !p.all_drm && !p.playlist);
        assert_eq!(p.filesize, Some(1000));
        // direct link: no formats listed → assume audio
        assert!(
            parse_probe(r#"{"title":"podcast_ep","ext":"mp3"}"#)
                .unwrap()
                .has_audio
        );
        // live / upcoming / post-live
        assert!(parse_probe(r#"{"is_live":true}"#).unwrap().live);
        assert!(
            parse_probe(r#"{"live_status":"is_upcoming"}"#)
                .unwrap()
                .live
        );
        assert!(!parse_probe(r#"{"live_status":"was_live"}"#).unwrap().live);
        // video only
        assert!(
            !parse_probe(r#"{"formats":[{"acodec":"none"},{"acodec":"none"}]}"#)
                .unwrap()
                .has_audio
        );
        // everything DRM
        assert!(
            parse_probe(r#"{"formats":[{"acodec":"aac","has_drm":true}]}"#)
                .unwrap()
                .all_drm
        );
        // playlist → first item
        let p = parse_probe(r#"{"_type":"playlist","title":"List","entries":[null,{"title":"First","duration":60}]}"#).unwrap();
        assert!(p.playlist);
        assert_eq!(p.title.as_deref(), Some("First"));
        assert!(parse_probe(r#"{"_type":"playlist","entries":[]}"#).is_err());
        assert!(parse_probe("not json").is_err());
    }

    #[test]
    fn classifies_real_world_messages() {
        let cases: &[(&str, K)] = &[
            ("ERROR: [youtube] abc: Private video. Sign in if you've been granted access to this video", K::NeedsLogin),
            ("ERROR: [youtube] abc: Sign in to confirm your age. This video may be inappropriate for some users.", K::NeedsLogin),
            ("ERROR: [youtube] abc: Join this channel to get access to members-only content like this video", K::NeedsLogin),
            ("ERROR: [youtube] abc: Sign in to confirm you’re not a bot. Use --cookies-from-browser", K::NeedsLogin),
            ("ERROR: could not find chrome cookies database in \"/Users/me/Library/Application Support/Google/Chrome\"", K::Cookies),
            ("ERROR: [Errno 1] Operation not permitted: '/Users/me/Library/Containers/com.apple.Safari/Data/Library/Cookies/Cookies.binarycookies'", K::Cookies),
            ("ERROR: Failed to decrypt cookie (AES-GCM) because the Keychain item could not be read", K::Cookies),
            ("ERROR: [DRM] The requested site is known to use DRM protection. It will NOT be supported.", K::Drm),
            ("ERROR: [youtube] abc: This video is DRM protected", K::Drm),
            ("ERROR: [youtube] abc: This live event will begin in 2 hours.", K::Live),
            ("ERROR: [generic] 404: Unable to download webpage: HTTP Error 404: Not Found (caused by <HTTPError 404: Not Found>)", K::NotFound),
            ("ERROR: [youtube] abc: Video unavailable", K::NotFound),
            ("ERROR: [youtube] abc: Requested format is not available. Use --list-formats for a list of available formats", K::NoAudio),
            ("ERROR: unable to write data: [Errno 28] No space left on device", K::DiskFull),
            ("ERROR: [generic] x: Unable to download webpage: HTTPConnection(host='127.0.0.1', port=1): Failed to establish a new connection: [Errno 111] Connection refused", K::Network),
            ("ERROR: [generic] x: Unable to download webpage: <urlopen error [Errno -2] Name or service not known>", K::Network),
            ("ERROR: [generic] x: Unable to download webpage: Failed to perform, curl: (7) Failed to connect to 127.0.0.1:1 after 2 ms: Could not connect to server.", K::Network),
            ("ERROR: [generic] x: Unable to download webpage: Failed to perform, curl: (6) Could not resolve host: nonexistent.invalid", K::Network),
            ("ERROR: Unsupported URL: http://127.0.0.1:1/html/page", K::Extractor),
            ("ERROR: [youtube] abc: Unable to extract player response; please report this issue", K::Extractor),
            ("ERROR: unable to download video data: HTTP Error 403: Forbidden", K::Extractor),
        ];
        for (text, want) in cases {
            assert_eq!(classify(text).kind, *want, "{text}");
        }
        // every non-extractor verdict has a human message that doesn't mention yt-dlp internals
        assert!(classify("Private video")
            .message
            .contains("Use browser cookies"));
        assert!(classify("[drm]").message.contains("DRM"));
    }

    #[test]
    fn only_extractor_errors_are_retryable() {
        for k in [
            K::Live,
            K::NeedsLogin,
            K::Drm,
            K::NoAudio,
            K::NotFound,
            K::Network,
            K::DiskFull,
            K::Cancelled,
            K::InvalidUrl,
        ] {
            assert!(!k.is_retryable(), "{k:?}");
        }
        assert!(K::Extractor.is_retryable());
    }
}
