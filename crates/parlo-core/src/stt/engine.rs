//! The STT engine seam (BRIEF §5.2 principle 5). Implementations: [`super::whisper::WhisperEngine`].

use crate::error::Result;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub struct SegmentOut {
    /// seconds relative to the start of the audio passed in
    pub start_s: f64,
    pub end_s: f64,
    pub text: String,
    pub no_speech_prob: f32,
    /// mean token log-probability (0 = certain; more negative = less confident)
    pub avg_logprob: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChunkTranscript {
    pub segments: Vec<SegmentOut>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LangGuess {
    pub code: String,
    pub prob: f32,
}

#[derive(Debug, Clone)]
pub struct TranscribeOpts {
    /// ISO-639-1 code; `None` lets the engine decide (the pipeline always passes the locked language).
    pub language: Option<String>,
    /// text from the previous chunk, used as decoder context when enabled
    pub initial_prompt: Option<String>,
    pub threads: usize,
}

pub trait SttEngine: Send {
    fn id(&self) -> &str;
    /// Detect the spoken language of a ≤ 30 s mono 16 kHz clip.
    fn detect_language(&mut self, samples: &[f32], threads: usize) -> Result<LangGuess>;
    /// Transcribe a ≤ 30 s mono 16 kHz clip. Must return `Error::Cancelled` promptly when `abort` is set.
    fn transcribe(
        &mut self,
        samples: &[f32],
        opts: &TranscribeOpts,
        abort: &Arc<AtomicBool>,
    ) -> Result<ChunkTranscript>;
}
