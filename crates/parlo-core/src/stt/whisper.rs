//! whisper.cpp through `whisper-rs` (BRIEF D3): Whisper large-v3-turbo family + Silero VAD.
//! Metal is enabled by the `whisper-metal` feature on macOS; on Linux this is the CPU path used for tests/benchmarks.

use super::engine::{ChunkTranscript, LangGuess, SegmentOut, SttEngine, TranscribeOpts};
use super::vad::{Span, SpeechDetector};
use crate::error::{Error, Result};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
    WhisperVadContext, WhisperVadContextParams, WhisperVadParams,
};

#[derive(Debug, Clone)]
pub struct WhisperConfig {
    pub use_gpu: bool,
    /// 1 = greedy, >1 = beam search
    pub beam_size: i32,
    pub temperature_inc: f32,
    pub no_speech_thold: f32,
    pub logprob_thold: f32,
    pub entropy_thold: f32,
}

impl Default for WhisperConfig {
    fn default() -> Self {
        // Chosen from the §6.2 benchmark (see docs/DECISIONS.md); BRIEF §7.2 step 4.
        Self {
            use_gpu: cfg!(feature = "whisper-metal"),
            beam_size: 1,
            temperature_inc: 0.2,
            no_speech_thold: 0.6,
            logprob_thold: -1.0,
            entropy_thold: 2.4,
        }
    }
}

/// whisper.cpp/ggml log to stderr by default; route everything through `tracing` once per process.
fn install_log_hooks() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(whisper_rs::install_logging_hooks);
}

pub struct WhisperEngine {
    id: String,
    // `state` borrows nothing from ctx in whisper-rs (it holds an Arc); keep ctx alive anyway for clarity
    _ctx: WhisperContext,
    state: WhisperState,
    cfg: WhisperConfig,
}

fn werr(e: impl std::fmt::Display) -> Error {
    Error::Other(format!("whisper: {e}"))
}

impl WhisperEngine {
    pub fn load(model: &Path, cfg: WhisperConfig) -> Result<Self> {
        if !model.exists() {
            return Err(Error::Other(format!(
                "Speech model not found: {}. Run scripts/fetch-models.sh.",
                model.display()
            )));
        }
        install_log_hooks();
        let mut cp = WhisperContextParameters::default();
        cp.use_gpu(cfg.use_gpu);
        let ctx = WhisperContext::new_with_params(
            model
                .to_str()
                .ok_or_else(|| Error::Other("model path is not UTF-8".into()))?,
            cp,
        )
        .map_err(werr)?;
        let state = ctx.create_state().map_err(werr)?;
        let id = model
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "whisper".into());
        Ok(Self {
            id,
            _ctx: ctx,
            state,
            cfg,
        })
    }
}

impl SttEngine for WhisperEngine {
    fn id(&self) -> &str {
        &self.id
    }

    fn detect_language(&mut self, samples: &[f32], threads: usize) -> Result<LangGuess> {
        self.state
            .pcm_to_mel(samples, threads.max(1))
            .map_err(werr)?;
        let (id, probs) = self.state.lang_detect(0, threads.max(1)).map_err(werr)?;
        let code = whisper_rs::get_lang_str(id).unwrap_or("en").to_string();
        Ok(LangGuess {
            code,
            prob: probs.get(id as usize).copied().unwrap_or(0.0),
        })
    }

    fn transcribe(
        &mut self,
        samples: &[f32],
        opts: &TranscribeOpts,
        abort: &Arc<AtomicBool>,
    ) -> Result<ChunkTranscript> {
        let strategy = if self.cfg.beam_size > 1 {
            SamplingStrategy::BeamSearch {
                beam_size: self.cfg.beam_size,
                patience: -1.0,
            }
        } else {
            SamplingStrategy::Greedy { best_of: 1 }
        };
        let mut p = FullParams::new(strategy);
        p.set_n_threads(opts.threads.max(1) as i32);
        p.set_language(opts.language.as_deref());
        p.set_print_special(false);
        p.set_print_progress(false);
        p.set_print_realtime(false);
        p.set_print_timestamps(false);
        p.set_suppress_blank(true);
        p.set_suppress_nst(true);
        p.set_no_context(opts.initial_prompt.is_none());
        p.set_temperature(0.0);
        p.set_temperature_inc(self.cfg.temperature_inc);
        p.set_no_speech_thold(self.cfg.no_speech_thold);
        p.set_logprob_thold(self.cfg.logprob_thold);
        p.set_entropy_thold(self.cfg.entropy_thold);
        if let Some(prompt) = &opts.initial_prompt {
            p.set_initial_prompt(prompt);
        }
        // NOTE: whisper-rs 0.16.0's `set_abort_callback_safe` is broken (it boxes the closure twice but its trampoline is
        // instantiated for the closure type, so the callback reads garbage and `full` fails with error -6). We install
        // our own C trampoline over the `AtomicBool` instead; `abort` outlives the `full` call below.
        unsafe extern "C" fn abort_trampoline(user_data: *mut std::ffi::c_void) -> bool {
            (*(user_data as *const AtomicBool)).load(Ordering::Relaxed)
        }
        unsafe {
            p.set_abort_callback(Some(abort_trampoline));
            p.set_abort_callback_user_data(Arc::as_ptr(abort) as *mut std::ffi::c_void);
        }
        let res = self.state.full(p, samples);
        if abort.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        res.map_err(werr)?;
        let mut segments = Vec::new();
        for seg in self.state.as_iter() {
            let text = seg.to_str_lossy().map_err(werr)?.trim().to_string();
            // mean log-prob over text tokens (skip special/timestamp tokens)
            let (mut sum, mut n) = (0f32, 0u32);
            for i in 0..seg.n_tokens() {
                if let Some(t) = seg.get_token(i) {
                    let s = t.to_str_lossy().map(|c| c.to_string()).unwrap_or_default();
                    if s.starts_with("[_") || s.starts_with("<|") || s.is_empty() {
                        continue;
                    }
                    let pr = t.token_probability().max(1e-9);
                    sum += pr.ln();
                    n += 1;
                }
            }
            segments.push(SegmentOut {
                start_s: seg.start_timestamp() as f64 / 100.0,
                end_s: seg.end_timestamp() as f64 / 100.0,
                text,
                no_speech_prob: seg.no_speech_probability(),
                avg_logprob: if n > 0 { sum / n as f32 } else { 0.0 },
            });
        }
        Ok(ChunkTranscript { segments })
    }
}

/// Silero VAD (GGML) through whisper.cpp.
pub struct SileroVad {
    ctx: WhisperVadContext,
    params: WhisperVadParams,
}

impl SileroVad {
    /// `min_silence_ms`: silence needed to end a speech span (BRIEF §7.2: cut at silences ≥ 300 ms).
    pub fn load(model: &Path, min_silence_ms: i32) -> Result<Self> {
        if !model.exists() {
            return Err(Error::Other(format!(
                "VAD model not found: {}. Run scripts/fetch-models.sh.",
                model.display()
            )));
        }
        install_log_hooks();
        let mut cp = WhisperVadContextParams::default();
        cp.set_n_threads(1);
        cp.set_use_gpu(false);
        let ctx = WhisperVadContext::new(
            model
                .to_str()
                .ok_or_else(|| Error::Other("VAD path is not UTF-8".into()))?,
            cp,
        )
        .map_err(werr)?;
        let mut params = WhisperVadParams::new();
        params.set_threshold(0.5);
        params.set_min_speech_duration(250);
        params.set_min_silence_duration(min_silence_ms);
        params.set_speech_pad(100);
        Ok(Self { ctx, params })
    }
}

impl SpeechDetector for SileroVad {
    fn detect(&mut self, samples: &[f32]) -> Result<Vec<Span>> {
        let segs = self
            .ctx
            .segments_from_samples(self.params, samples)
            .map_err(werr)?;
        // whisper-rs documents centiseconds, but the segment times are seconds-scaled in whisper.cpp ≥ 1.8 (verified by
        // the `vad_units` test against known tone positions); keep one conversion point here.
        Ok(segs
            .map(|s| Span {
                start_s: s.start as f64 / super::vad_unit_divisor(),
                end_s: s.end as f64 / super::vad_unit_divisor(),
            })
            .collect())
    }
}
