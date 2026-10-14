//! Speech-to-text (BRIEF §7): VAD → ≤ 30 s chunks → language lock → whisper.cpp → post-filters → paragraphs,
//! with per-chunk checkpoints so a crash or quit resumes from the last finished chunk.

pub mod assemble;
pub mod checkpoint;
pub mod chunker;
pub mod engine;
pub mod filters;
pub mod pipeline;
pub mod vad;
#[cfg(feature = "whisper")]
pub mod whisper;

pub use engine::{ChunkTranscript, LangGuess, SegmentOut, SttEngine, TranscribeOpts};
pub use pipeline::{run_transcription, SttEvent, SttJob, SttSink, Transcript};
pub use vad::SpeechDetector;

/// Divisor converting whisper-rs VAD segment timestamps to seconds (see `whisper::SileroVad`).
pub(crate) fn vad_unit_divisor() -> f64 {
    VAD_UNIT_DIVISOR
}
/// Calibrated by tests/stt_vad.rs against known tone positions: whisper.cpp reports centiseconds.
const VAD_UNIT_DIVISOR: f64 = 100.0;
