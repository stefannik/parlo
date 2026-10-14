//! The TTS engine seam (BRIEF D4, §5.2 principle 5). Implementations: PocketTTS via the patched Candle port, or the
//! Python sidecar fallback; switching is a one-line change where the engine is constructed.

use crate::error::Result;
use std::sync::atomic::AtomicBool;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality {
    /// `lsd_decode_steps = 4` — ctric parity (default)
    Standard,
    /// `lsd_decode_steps = 1`
    Fast,
}

impl Quality {
    pub fn lsd_decode_steps(self) -> usize {
        match self {
            Quality::Standard => 4,
            Quality::Fast => 1,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Quality::Standard => "standard",
            Quality::Fast => "fast",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceInfo {
    pub id: String,
    pub label: String,
}

pub trait TtsEngine: Send {
    /// Stable engine id, part of the output cache key (e.g. `pockettts-candle`).
    fn id(&self) -> &str;
    /// Model revision string, part of the output cache key.
    fn model_revision(&self) -> &str;
    fn sample_rate(&self) -> u32;
    /// Voices verified to exist on disk (BRIEF §9.1: missing voices are hidden).
    fn voices(&self) -> Vec<VoiceInfo>;
    /// Synthesize `text` (already normalized) and stream mono f32 PCM at `sample_rate()` to `sink` as it is produced.
    /// Must return `Error::Cancelled` promptly when `abort` is set.
    fn synthesize(
        &mut self,
        text: &str,
        voice: &str,
        quality: Quality,
        sink: &mut dyn FnMut(&[f32]) -> Result<()>,
        abort: &AtomicBool,
    ) -> Result<()>;
}
