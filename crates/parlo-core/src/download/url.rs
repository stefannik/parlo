//! URL handling for "Transcribe from URL": validation (http/https only), a readable fallback title, and the
//! "is this plainly an audio file" test that gates the curl fallback. Pure string work — no sockets here.

use super::error::{DownloadError, DownloadErrorKind};

/// Extensions that mean "this URL is the audio file itself" (podcast enclosures, direct links).
pub const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "m4a", "aac", "wav", "flac", "ogg", "oga", "opus", "webm", "weba", "aif", "aiff", "caf",
    "mp4", "m4b",
];

pub const MAX_URL_LEN: usize = 2048;

/// Trim and validate a user-typed/pasted URL. Only `http`/`https` with a host are accepted; the returned string is the
/// normalized form (`url::Url` serialization), which is what gets passed to the downloader after `--`.
pub fn validate_url(input: &str) -> Result<String, DownloadError> {
    let s = input.trim();
    let bad = |m: &str| DownloadError::new(DownloadErrorKind::InvalidUrl, m, s.to_string());
    if s.is_empty() {
        return Err(bad("Paste a link first."));
    }
    if s.len() > MAX_URL_LEN {
        return Err(bad("That link is too long."));
    }
    if s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(bad(
            "That doesn’t look like a single web link (it contains spaces).",
        ));
    }
    let u = url::Url::parse(s).map_err(|_| {
        bad("That doesn’t look like a web link. It should start with http:// or https://")
    })?;
    match u.scheme() {
        "http" | "https" => {}
        other => {
            return Err(bad(&format!(
                "Only http:// and https:// links are supported (this one is {other}://)."
            )))
        }
    }
    if u.host_str().is_none_or(str::is_empty) {
        return Err(bad("That link has no website name."));
    }
    Ok(u.to_string())
}

/// Quick yes/no for pasted/dropped text: is it a valid downloadable link?
pub fn is_valid_url(input: &str) -> bool {
    validate_url(input).is_ok()
}

/// The URL without credentials or query string: safe to show in logs and error details.
pub fn redact(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut u) => {
            let _ = u.set_username("");
            let _ = u.set_password(None);
            u.set_query(None);
            u.set_fragment(None);
            u.to_string()
        }
        Err(_) => "(invalid url)".into(),
    }
}

/// `host` of a URL, without a leading `www.`.
pub fn host_of(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    let h = u.host_str()?;
    Some(h.strip_prefix("www.").unwrap_or(h).to_string())
}

/// Last non-empty path segment, percent-decoded (`Episode%2012.mp3` → `Episode 12.mp3`).
pub fn file_name_of(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    let seg = u.path_segments()?.rfind(|s| !s.is_empty())?;
    let dec = percent_decode(seg);
    (!dec.is_empty()).then_some(dec)
}

/// Extension (lowercase, no dot) of the URL's path, if any.
pub fn extension_of(url: &str) -> Option<String> {
    let name = file_name_of(url)?;
    let (_, ext) = name.rsplit_once('.')?;
    let ext = ext.to_ascii_lowercase();
    (!ext.is_empty() && ext.len() <= 5 && ext.chars().all(|c| c.is_ascii_alphanumeric()))
        .then_some(ext)
}

pub fn looks_like_audio_url(url: &str) -> bool {
    extension_of(url).is_some_and(|e| AUDIO_EXTENSIONS.contains(&e.as_str()))
}

/// Job name when the site gives no title: `file name (host)`, or just the host.
pub fn fallback_title(url: &str) -> String {
    let host = host_of(url).unwrap_or_else(|| "link".into());
    match file_name_of(url) {
        Some(f) => format!("{f} ({host})"),
        None => host,
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_http_and_https() {
        assert_eq!(
            validate_url("  https://www.youtube.com/watch?v=abc  ").unwrap(),
            "https://www.youtube.com/watch?v=abc"
        );
        assert!(validate_url("http://127.0.0.1:8080/a.mp3").is_ok());
        assert!(validate_url("https://example.com").is_ok());
    }

    #[test]
    fn rejects_everything_else() {
        for bad in [
            "",
            "   ",
            "youtube.com/watch?v=abc",
            "ftp://example.com/a.mp3",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "https://",
            "https://exa mple.com",
            "two https://a.com links",
            "-o /etc/passwd",
            "data:audio/mp3;base64,AAAA",
        ] {
            let e = validate_url(bad).expect_err(bad);
            assert_eq!(e.kind, DownloadErrorKind::InvalidUrl, "{bad}");
        }
        assert!(validate_url(&format!("https://a.com/{}", "x".repeat(3000))).is_err());
    }

    #[test]
    fn titles_fall_back_to_file_and_host() {
        assert_eq!(
            fallback_title("https://cdn.example.com/pod/Episode%2012.mp3?x=1"),
            "Episode 12.mp3 (cdn.example.com)"
        );
        assert_eq!(fallback_title("https://www.example.com/"), "example.com");
        assert_eq!(fallback_title("https://example.com"), "example.com");
    }

    #[test]
    fn audio_extension_detection() {
        assert!(looks_like_audio_url("https://a.com/x/ep.MP3?token=1"));
        assert!(looks_like_audio_url("https://a.com/x/ep.m4a"));
        assert!(!looks_like_audio_url("https://a.com/watch?v=1"));
        assert!(!looks_like_audio_url("https://a.com/page.html"));
        assert!(!looks_like_audio_url("https://a.com/"));
    }

    #[test]
    fn redaction_drops_credentials_and_query() {
        assert_eq!(
            redact("https://user:pw@a.com/p/ep.mp3?token=SECRET#t"),
            "https://a.com/p/ep.mp3"
        );
    }
}
