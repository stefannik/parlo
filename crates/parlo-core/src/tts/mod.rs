//! Text-to-speech (BRIEF §9): engine seam, long-text pipeline (segments, silences, cache, resume), previews.

pub mod engine;
pub mod pipeline;
#[cfg(feature = "tts-pocket")]
pub mod pocket;

pub use engine::{Quality, TtsEngine, VoiceInfo};
pub use pipeline::{output_id, run_tts, TtsEvent, TtsJob, TtsOutput, TtsSink};
