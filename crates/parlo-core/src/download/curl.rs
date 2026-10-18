//! `/usr/bin/curl` as the last resort for **direct media links** (an audio file, e.g. a podcast RSS enclosure): used only
//! when no yt-dlp works, and only if the server says `Content-Type: audio/*` or the URL ends in an audio extension.

use super::error::{DownloadError, DownloadErrorKind as K};
use super::proc::{self, Live, RunError};
use super::{DownloadEvent, FetchCtx, FetchOut, Fetcher, ToolKind};
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

pub struct CurlFetcher {
    pub path: PathBuf,
}

/// What the (last) response of a redirect chain says about the resource.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HeadInfo {
    pub status: u16,
    pub content_type: Option<String>,
    pub total_bytes: Option<u64>,
    pub file_name: Option<String>,
    pub effective_url: Option<String>,
}

/// Parse the output of `curl -D - -o /dev/null -w '\nPARLO_URL %{url_effective}\n'`: header blocks of every hop, the
/// last one wins.
pub fn parse_head(text: &str) -> HeadInfo {
    let mut cur = HeadInfo::default();
    for raw in text.lines() {
        let l = raw.trim_end_matches('\r');
        if let Some(rest) = l.strip_prefix("PARLO_URL ") {
            cur.effective_url = Some(rest.trim().to_string());
        } else if l.starts_with("HTTP/") {
            // new hop: reset (keep the effective url, which is printed last anyway)
            cur = HeadInfo {
                status: l
                    .split_whitespace()
                    .nth(1)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0),
                ..HeadInfo::default()
            };
        } else if let Some((k, v)) = l.split_once(':') {
            let v = v.trim();
            match k.trim().to_ascii_lowercase().as_str() {
                "content-type" => {
                    cur.content_type =
                        Some(v.split(';').next().unwrap_or(v).trim().to_ascii_lowercase())
                }
                "content-range" => {
                    // bytes 0-0/12345
                    cur.total_bytes = v.rsplit('/').next().and_then(|n| n.parse().ok());
                }
                "content-length" if cur.total_bytes.is_none() && cur.status != 206 => {
                    cur.total_bytes = v.parse().ok()
                }
                "content-disposition" => {
                    if let Some(i) = v.to_ascii_lowercase().find("filename=") {
                        let f = v[i + 9..].trim().trim_matches(|c| c == '"' || c == '\'');
                        let f = f.split(';').next().unwrap_or(f).trim_matches('"');
                        if !f.is_empty() {
                            cur.file_name = Some(f.to_string());
                        }
                    }
                }
                _ => {}
            }
        }
    }
    cur
}

/// Is this plainly an audio file? (`audio/*`, or an audio extension on the URL / disposition file name.)
pub fn is_direct_audio(h: &HeadInfo, url: &str) -> bool {
    if h.content_type
        .as_deref()
        .is_some_and(|c| c.starts_with("audio/"))
    {
        return true;
    }
    let ext_ok = |name: &str| {
        name.rsplit_once('.').is_some_and(|(_, e)| {
            super::url::AUDIO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str())
        })
    };
    // application/octet-stream (or video/mp4 for .m4a/.mp4 podcast files) is fine when the name says audio
    let generic = h.content_type.as_deref().is_none_or(|c| {
        c.starts_with("application/octet-stream")
            || c.starts_with("binary/")
            || c.starts_with("video/mp4")
            || c.starts_with("video/webm")
    });
    generic
        && (h.file_name.as_deref().is_some_and(ext_ok)
            || super::url::looks_like_audio_url(h.effective_url.as_deref().unwrap_or(url))
            || super::url::looks_like_audio_url(url))
}

/// File extension to store the download under.
pub fn extension_for(h: &HeadInfo, url: &str) -> String {
    let by_type = h.content_type.as_deref().and_then(|c| match c {
        "audio/mpeg" | "audio/mp3" => Some("mp3"),
        "audio/mp4" | "audio/x-m4a" | "audio/m4a" => Some("m4a"),
        "audio/aac" | "audio/aacp" => Some("aac"),
        "audio/ogg" => Some("ogg"),
        "audio/opus" => Some("opus"),
        "audio/webm" => Some("webm"),
        "audio/wav" | "audio/x-wav" | "audio/wave" => Some("wav"),
        "audio/flac" | "audio/x-flac" => Some("flac"),
        "audio/aiff" | "audio/x-aiff" => Some("aiff"),
        _ => None,
    });
    if let Some(e) = by_type {
        return e.into();
    }
    h.file_name
        .as_deref()
        .and_then(|f| f.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()))
        .filter(|e| super::url::AUDIO_EXTENSIONS.contains(&e.as_str()))
        .or_else(|| super::url::extension_of(h.effective_url.as_deref().unwrap_or(url)))
        .filter(|e| super::url::AUDIO_EXTENSIONS.contains(&e.as_str()))
        .unwrap_or_else(|| "audio".into())
}

/// Parse a line of curl's default progress meter:
/// `  % Total    % Received % Xferd  Average Speed   Time    Time     Time  Current`
/// `100  110k  100  110k    0     0  60.9k      0  0:00:01  0:00:01 --:--:-- 60.9k`
pub fn parse_meter(line: &str) -> Option<(u64, Option<u64>, Option<f64>)> {
    let f: Vec<&str> = line.split_whitespace().collect();
    if f.len() < 12 || f[0].parse::<u32>().is_err() {
        return None;
    }
    let total = size(f[1]);
    let got = size(f[3])?;
    let speed = size(f[11]).map(|s| s as f64);
    Some((got, total.filter(|t| *t > 0), speed))
}

/// `110k`, `1.5M`, `12345`, `2G`
fn size(s: &str) -> Option<u64> {
    let (num, mul) = match s.chars().last()? {
        'k' | 'K' => (&s[..s.len() - 1], 1024.0),
        'M' => (&s[..s.len() - 1], 1024.0 * 1024.0),
        'G' => (&s[..s.len() - 1], 1024.0 * 1024.0 * 1024.0),
        'T' => (&s[..s.len() - 1], 1024.0_f64.powi(4)),
        'P' => (&s[..s.len() - 1], 1024.0_f64.powi(5)),
        c if c.is_ascii_digit() => (s, 1.0),
        _ => return None,
    };
    num.parse::<f64>().ok().map(|n| (n * mul) as u64)
}

pub fn head_args(url: &str) -> Vec<OsString> {
    [
        "-sS",
        "-L",
        "--max-redirs",
        "10",
        "--proto",
        "=http,https",
        "--proto-redir",
        "=http,https",
        "--max-time",
        "30",
        "-r",
        "0-0",
        "-o",
        "/dev/null",
        "-D",
        "-",
        "-w",
        "\nPARLO_URL %{url_effective}\n",
        "--",
        url,
    ]
    .iter()
    .map(OsString::from)
    .collect()
}

pub fn download_args(url: &str, out: &std::path::Path) -> Vec<OsString> {
    let mut a: Vec<OsString> = [
        "-L",
        "-f",
        "--max-redirs",
        "10",
        "--proto",
        "=http,https",
        "--proto-redir",
        "=http,https",
        "--connect-timeout",
        "30",
        "--retry",
        "2",
        "--retry-connrefused",
        "-o",
    ]
    .iter()
    .map(OsString::from)
    .collect();
    a.push(out.into());
    a.extend(["--".into(), url.into()]);
    a
}

impl Fetcher for CurlFetcher {
    fn kind(&self) -> ToolKind {
        ToolKind::Curl
    }

    fn fetch(
        &self,
        ctx: &FetchCtx,
        sink: &mut dyn FnMut(DownloadEvent),
        cancel: &AtomicBool,
    ) -> Result<FetchOut, DownloadError> {
        // a link that plainly isn't a file can't be handled by curl at all: don't even probe
        let (r, out) = proc::run_capture(
            proc::command(&self.path, &head_args(&ctx.url), None),
            cancel,
            Duration::from_secs(45),
        )
        .map_err(|e| match e {
            RunError::Cancelled => DownloadError::cancelled(),
            other => {
                DownloadError::new(K::NotApplicable, "curl didn’t answer", format!("{other:?}"))
            }
        })?;
        let head = parse_head(&out);
        if !r.status.success() && head.status == 0 {
            let err = r.stderr_text();
            let low = err.to_ascii_lowercase();
            let kind = if low.contains("could not resolve")
                || low.contains("connection refused")
                || low.contains("timed out")
                || low.contains("failed to connect")
            {
                K::Network
            } else {
                K::NotApplicable
            };
            return Err(DownloadError::new(
                kind,
                "Couldn’t reach the website. Check your internet connection and try again.",
                err,
            ));
        }
        match head.status {
            200 | 206 | 416 => {}
            404 | 410 => return Err(super::ytdlp::classify("HTTP Error 404")),
            _ => {
                return Err(DownloadError::new(
                    K::NotApplicable,
                    "The server refused the download.",
                    format!("HTTP {}", head.status),
                ))
            }
        }
        if !is_direct_audio(&head, &ctx.url) {
            return Err(DownloadError::new(
                K::NotApplicable,
                "This link is a web page, not an audio file.",
                format!(
                    "content-type: {}",
                    head.content_type.as_deref().unwrap_or("?")
                ),
            ));
        }
        let ext = extension_for(&head, &ctx.url);
        let title = head
            .file_name
            .clone()
            .map(|f| f.rsplit_once('.').map(|(s, _)| s.to_string()).unwrap_or(f))
            .unwrap_or_else(|| {
                super::url::fallback_title(head.effective_url.as_deref().unwrap_or(&ctx.url))
            });
        sink(DownloadEvent::Info {
            title: title.clone(),
            duration_s: None,
            playlist: false,
        });
        if let Some(need) = head.total_bytes {
            let free = crate::paths::free_disk_bytes(&ctx.stage_dir).unwrap_or(u64::MAX);
            if free < need.saturating_add(50 << 20) {
                return Err(super::ytdlp::classify("No space left on device"));
            }
        }
        let dest = ctx.stage_dir.join(format!("media.{ext}"));
        let mut total = head.total_bytes;
        let r = proc::run_streaming(
            proc::command(&self.path, &download_args(&ctx.url, &dest), None),
            Live::Stderr,
            cancel,
            None,
            &mut |l| {
                if let Some((got, t, speed)) = parse_meter(l) {
                    total = t.or(total);
                    let eta = match (total, speed) {
                        (Some(t), Some(s)) if s > 0.0 && t >= got => Some((t - got) as f64 / s),
                        _ => None,
                    };
                    sink(DownloadEvent::Progress {
                        downloaded: got,
                        total,
                        speed_bps: speed,
                        eta_s: eta,
                    });
                }
            },
        )
        .map_err(|e| match e {
            RunError::Cancelled => DownloadError::cancelled(),
            RunError::TimedOut => DownloadError::new(K::Network, "The download timed out.", ""),
            RunError::Spawn(io) => DownloadError::new(
                K::NotApplicable,
                "curl couldn’t be started.",
                io.to_string(),
            ),
        })?;
        if !r.status.success() {
            let err = r.stderr_text();
            return Err(match r.status.code() {
                Some(23) => super::ytdlp::classify("No space left on device"),
                Some(22) => super::ytdlp::classify("HTTP Error 404"),
                Some(6) | Some(7) | Some(28) | Some(35) | Some(56) => DownloadError::new(
                    K::Network,
                    "Couldn’t reach the website. Check your internet connection and try again.",
                    err,
                ),
                _ => DownloadError::new(K::Extractor, "The download failed.", err),
            });
        }
        if std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0) == 0 {
            return Err(DownloadError::new(
                K::Extractor,
                "The download was empty.",
                "",
            ));
        }
        Ok(FetchOut {
            path: dest,
            title,
            duration_s: None,
            tool: ToolKind::Curl,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn head_parsing_follows_redirects_and_takes_the_last_hop() {
        let t = "HTTP/1.1 302 Found\r\nLocation: /a.mp3\r\nContent-Length: 0\r\n\r\nHTTP/1.1 206 Partial Content\r\nContent-Type: audio/mpeg\r\nContent-Length: 1\r\nContent-Range: bytes 0-0/113136\r\nContent-Disposition: attachment; filename=\"Episode 12.mp3\"\r\n\r\n\nPARLO_URL http://h/a.mp3\n";
        let h = parse_head(t);
        assert_eq!(h.status, 206);
        assert_eq!(h.content_type.as_deref(), Some("audio/mpeg"));
        assert_eq!(h.total_bytes, Some(113136));
        assert_eq!(h.file_name.as_deref(), Some("Episode 12.mp3"));
        assert_eq!(h.effective_url.as_deref(), Some("http://h/a.mp3"));
    }

    #[test]
    fn head_with_plain_200_uses_content_length() {
        let h = parse_head(
            "HTTP/2 200\r\ncontent-type: Audio/MP4; charset=x\r\ncontent-length: 5000\r\n\r\n",
        );
        assert_eq!((h.status, h.total_bytes), (200, Some(5000)));
        assert_eq!(h.content_type.as_deref(), Some("audio/mp4"));
    }

    #[test]
    fn only_direct_audio_is_applicable() {
        let h = |ct: Option<&str>, f: Option<&str>| HeadInfo {
            status: 200,
            content_type: ct.map(Into::into),
            file_name: f.map(Into::into),
            ..HeadInfo::default()
        };
        assert!(is_direct_audio(
            &h(Some("audio/mpeg"), None),
            "https://a.com/stream"
        ));
        assert!(is_direct_audio(
            &h(Some("audio/x-whatever"), None),
            "https://a.com/x"
        ));
        assert!(is_direct_audio(
            &h(Some("application/octet-stream"), None),
            "https://a.com/ep.mp3?x=1"
        ));
        assert!(is_direct_audio(
            &h(None, Some("a.m4a")),
            "https://a.com/dl?id=1"
        ));
        assert!(is_direct_audio(
            &h(Some("video/mp4"), None),
            "https://a.com/pod.m4a"
        ));
        assert!(
            !is_direct_audio(&h(Some("text/html"), None), "https://a.com/ep.mp3"),
            "html is never audio, whatever the URL says"
        );
        assert!(!is_direct_audio(
            &h(Some("text/html"), None),
            "https://a.com/watch?v=1"
        ));
        assert!(!is_direct_audio(
            &h(Some("application/octet-stream"), None),
            "https://a.com/archive.zip"
        ));
        assert!(!is_direct_audio(
            &h(Some("video/mp4"), None),
            "https://a.com/movie.html"
        ));
    }

    #[test]
    fn extension_choice() {
        let h = |ct: &str| HeadInfo {
            content_type: Some(ct.into()),
            ..HeadInfo::default()
        };
        assert_eq!(extension_for(&h("audio/mpeg"), "https://a/x"), "mp3");
        assert_eq!(extension_for(&h("audio/mp4"), "https://a/x"), "m4a");
        assert_eq!(extension_for(&h("audio/x-m4a"), "https://a/x"), "m4a");
        assert_eq!(
            extension_for(&h("application/octet-stream"), "https://a/x/ep.OGG"),
            "ogg"
        );
        assert_eq!(
            extension_for(&h("application/octet-stream"), "https://a/x"),
            "audio"
        );
    }

    #[test]
    fn meter_lines() {
        let l = "100  110k  100  110k    0     0  60.9k      0  0:00:01  0:00:01 --:--:-- 60.9k";
        let (got, total, speed) = parse_meter(l).unwrap();
        assert_eq!(got, 110 * 1024);
        assert_eq!(total, Some(110 * 1024));
        assert_eq!(speed, Some((60.9 * 1024.0) as u64 as f64));
        let m = parse_meter(
            "  3 12.5M    3  400k    0     0  1500k      0  0:00:08  0:00:00  0:00:08 1500k",
        )
        .unwrap();
        assert_eq!(m.0, 400 * 1024);
        assert_eq!(m.1, Some((12.5 * 1024.0 * 1024.0) as u64));
        assert!(parse_meter("  % Total    % Received % Xferd  Average Speed").is_none());
        assert!(parse_meter("curl: (22) The requested URL returned error: 404").is_none());
    }

    #[test]
    fn args_are_hardened() {
        let a: Vec<String> =
            download_args("https://a.com/x.mp3", std::path::Path::new("/s/media.mp3"))
                .iter()
                .map(|s| s.to_string_lossy().into_owned())
                .collect();
        assert!(a.windows(2).any(|w| w == ["--proto", "=http,https"]));
        assert!(
            a.iter().any(|x| x == "-f"),
            "HTTP errors must fail the download"
        );
        assert_eq!(&a[a.len() - 2..], ["--", "https://a.com/x.mp3"]);
    }
}
