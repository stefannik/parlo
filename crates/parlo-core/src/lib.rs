//! Parlo core: all logic, no Tauri dependency (BRIEF §5.1).
pub mod audio;
pub mod download;
pub mod error;
pub mod paths;

pub use error::{Error, Result};

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
pub mod jobs;
pub mod logging;
pub mod models;
pub mod platform;
pub mod recorder;
pub mod store;
pub mod stt;
pub mod text;
pub mod tts;
