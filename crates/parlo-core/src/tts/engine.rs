//! The TTS engine seam (BRIEF D4, §5.2 principle 5). Implementations: PocketTTS via the patched Candle port, or the
//! Python sidecar fallback; switching is a one-line change where the engine is constructed.

use crate::error::Result;
use std::sync::atomic::AtomicBool;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality {
    /// Kyutai's default: one sampler step (what the 2026-09 weights were evaluated with)
    Standard,
    /// four sampler steps (ctric's old setting): slower, not measurably better in our listening tests
    High,
}

impl Quality {
    pub fn lsd_decode_steps(self) -> usize {
        match self {
            Quality::Standard => 1,
            Quality::High => 4,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Quality::Standard => "standard",
            Quality::High => "high",
        }
    }
    /// Settings/IPC string → quality (`"fast"` is the legacy name of the cheap setting = Standard).
    pub fn parse(s: &str) -> Self {
        if s == "high" {
            Quality::High
        } else {
            Quality::Standard
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceInfo {
    pub id: String,
    pub label: String,
    /// ISO-639-1 language the voice speaks (`en`, `es`)
    pub language: String,
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
