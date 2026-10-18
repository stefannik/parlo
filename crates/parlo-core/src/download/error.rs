//! Why a download failed, in terms the UI can act on. `message` is shown to the user as-is; `detail` is the tool's own
//! output tail ("Show details").

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DownloadErrorKind {
    /// not an http(s) link
    InvalidUrl,
    /// the "Allow downloads from URLs" switch is off
    Disabled,
    Cancelled,
    /// live stream or an upcoming premiere: refused before downloading
    Live,
    /// private / members-only / age-restricted / login wall → suggest the cookies option
    NeedsLogin,
    /// DRM-protected or a site that is known not to work (Spotify, Netflix …)
    Drm,
    /// the page has no audio stream
    NoAudio,
    /// 404 / gone / video unavailable
    NotFound,
    /// no connection, DNS failure, timeout
    Network,
    DiskFull,
    /// no downloader at all (bundled missing, no system yt-dlp, not a direct file)
    NoDownloader,
    /// the downloader ran but couldn't handle this link (extractor/format/HTTP error): the system copy may do better
    Extractor,
    /// this tool can't handle the link at all and shouldn't be reported (e.g. curl on a web page)
    NotApplicable,
}

impl DownloadErrorKind {
    /// Machine-readable code for the UI (`IpcError::code`).
    pub fn code(self) -> &'static str {
        match self {
            DownloadErrorKind::InvalidUrl => "invalid_url",
            DownloadErrorKind::Disabled => "download_disabled",
            DownloadErrorKind::Cancelled => "cancelled",
            DownloadErrorKind::Live => "download_live",
            DownloadErrorKind::NeedsLogin => "download_login",
            DownloadErrorKind::Drm => "download_drm",
            DownloadErrorKind::NoAudio => "download_no_audio",
            DownloadErrorKind::NotFound => "download_not_found",
            DownloadErrorKind::Network => "download_network",
            DownloadErrorKind::DiskFull => "disk_space",
            DownloadErrorKind::NoDownloader | DownloadErrorKind::Extractor => "download_failed",
            DownloadErrorKind::NotApplicable => "download_failed",
        }
    }

    /// May the next downloader in the chain do better? Everything else is a verdict about the link itself.
    pub fn is_retryable(self) -> bool {
        matches!(
            self,
            DownloadErrorKind::Extractor | DownloadErrorKind::NotApplicable
        )
    }
}

#[derive(Debug, Clone)]
pub struct DownloadError {
    pub kind: DownloadErrorKind,
    pub message: String,
    pub detail: String,
}

impl DownloadError {
    pub fn new(
        kind: DownloadErrorKind,
        message: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            message: message.into(),
            detail: detail.into(),
        }
    }

    pub fn cancelled() -> Self {
        Self::new(DownloadErrorKind::Cancelled, "Cancelled", "")
    }

    /// The closing message when nothing in the chain could handle the link.
    pub fn no_downloader(detail: impl Into<String>) -> Self {
        Self::new(
            DownloadErrorKind::NoDownloader,
            "Couldn’t download this link. Install or update yt-dlp: `brew install yt-dlp` / `brew upgrade yt-dlp`.",
            detail,
        )
    }
}

impl fmt::Display for DownloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for DownloadError {}

impl From<DownloadError> for crate::error::Error {
    fn from(e: DownloadError) -> Self {
        if e.kind == DownloadErrorKind::Cancelled {
            return crate::error::Error::Cancelled;
        }
        crate::error::Error::Other(e.message)
    }
}
