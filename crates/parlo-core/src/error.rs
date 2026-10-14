//! Error types shared across parlo-core. User-facing messages are written for humans (shown in the UI).

use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, Error>;

/// Why an audio input file could not be prepared (BRIEF §7.1: "reject with a clear message").
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("The file is empty.")]
    Empty,
    #[error("This file doesn't contain an audio track.")]
    NoAudioTrack,
    #[error("The audio has zero length.")]
    ZeroDuration,
    #[error("This file is protected (DRM) and can't be read.")]
    Drm,
    #[error("Parlo can't read this file format ({0}).")]
    Unsupported(String),
    #[error("The file is damaged or unreadable: {0}")]
    Corrupt(String),
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Decode(#[from] DecodeError),
    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("I/O error: {0}")]
    IoBare(#[from] std::io::Error),
    #[error("Audio device error: {0}")]
    Device(String),
    #[error("Microphone access is not allowed. Open System Settings → Privacy & Security → Microphone and enable Parlo.")]
    MicPermission,
    #[error("Not enough free disk space: {free_mb} MB free, {needed_mb} MB needed.")]
    DiskSpace { free_mb: u64, needed_mb: u64 },
    #[error("Encoding failed: {0}")]
    Encode(String),
    #[error("The operation was cancelled.")]
    Cancelled,
    #[error("Invalid state: {0}")]
    State(String),
    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Other(format!("JSON error: {e}"))
    }
}
impl From<hound::Error> for Error {
    fn from(e: hound::Error) -> Self {
        Error::Encode(e.to_string())
    }
}
