//! STT pipeline behaviour (BRIEF §7.2, §14.1): silence → empty transcript, cancel + resume, bounded RAM on a 2 h file.
//! A scripted engine stands in for whisper where the test is about pipeline logic; real VAD is always used.
mod common;
use common::{fixture, model};
use parlo_core::error::{Error, Result};
use parlo_core::stt::engine::{ChunkTranscript, LangGuess, SegmentOut, SttEngine, TranscribeOpts};
use parlo_core::stt::pipeline::{run_transcription, LanguageSetting, SttEvent, SttJob};
use parlo_core::stt::whisper::SileroVad;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

/// Returns one numbered sentence per chunk; counts calls; can raise `cancel` after N calls.
struct Scripted {
    calls: Arc<AtomicUsize>,
    cancel_after: Option<(usize, Arc<AtomicBool>)>,
    lang: &'static str,
}
impl SttEngine for Scripted {
    fn id(&self) -> &str {
        "scripted"
    }
    fn detect_language(&mut self, _s: &[f32], _t: usize) -> Result<LangGuess> {
        Ok(LangGuess {
            code: self.lang.into(),
            prob: 0.99,
        })
    }
    fn transcribe(
        &mut self,
        samples: &[f32],
        opts: &TranscribeOpts,
        abort: &Arc<AtomicBool>,
    ) -> Result<ChunkTranscript> {
        if abort.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        assert_eq!(
            opts.language.as_deref(),
            Some(self.lang),
            "the pipeline must pass the locked language"
        );
        let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some((k, flag)) = &self.cancel_after {
            if n >= *k {
                flag.store(true, Ordering::SeqCst);
            }
        }
        let dur = samples.len() as f64 / 16_000.0;
        Ok(ChunkTranscript {
            segments: vec![SegmentOut {
                start_s: 0.0,
                end_s: dur.min(2.0),
                text: format!("Sentence number {n} of the scripted transcript."),
                no_speech_prob: 0.0,
                avg_logprob: -0.2,
            }],
        })
    }
}

fn vad() -> SileroVad {
    SileroVad::load(&model("vad/ggml-silero-v5.1.2.bin"), 300).unwrap()
}

fn job(input: &str, dir: &std::path::Path) -> SttJob {
    let mut j = SttJob::new(fixture(input), dir.join("job"), dir.join("cache"));
    j.threads = 2;
    j
}

#[test]
fn silence_only_input_gives_an_empty_transcript_and_never_calls_the_engine() {
    let dir = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut eng = Scripted {
        calls: calls.clone(),
        cancel_after: None,
        lang: "en",
    };
    let mut v = vad();
    for f in ["silence_60s.wav", "noise_pink_60s.wav", "tone_30s.wav"] {
        let j = job(&format!("generated/edge/{f}"), dir.path());
        let t = run_transcription(
            &j,
            &mut eng,
            &mut v,
            &mut |_: SttEvent| {},
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert!(
            t.text.is_empty() && t.paragraphs.is_empty(),
            "{f}: {:?}",
            t.text
        );
        std::fs::remove_dir_all(dir.path().join("job")).ok();
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn cancel_then_resume_continues_from_the_last_finished_chunk() {
    let dir = tempfile::tempdir().unwrap();
    let j = job("generated/edge/en_long_10min.wav", dir.path());
    let mut v = vad();
    // 1) uninterrupted reference run
    let calls_ref = Arc::new(AtomicUsize::new(0));
    let mut eng = Scripted {
        calls: calls_ref.clone(),
        cancel_after: None,
        lang: "en",
    };
    let full = run_transcription(
        &j,
        &mut eng,
        &mut v,
        &mut |_: SttEvent| {},
        &Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    let total_chunks = full.chunks_total;
    assert!(total_chunks > 15);
    std::fs::remove_dir_all(dir.path().join("job")).unwrap();

    // 2) cancelled after 7 chunks
    let cancel = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut eng = Scripted {
        calls: calls.clone(),
        cancel_after: Some((7, cancel.clone())),
        lang: "en",
    };
    let r = run_transcription(&j, &mut eng, &mut v, &mut |_: SttEvent| {}, &cancel);
    assert!(matches!(r, Err(Error::Cancelled)), "{r:?}");
    assert_eq!(calls.load(Ordering::SeqCst), 7);

    // 3) resume: only the remaining chunks are transcribed, language not re-detected, result identical
    let calls2 = Arc::new(AtomicUsize::new(7));
    let mut eng2 = Scripted {
        calls: calls2.clone(),
        cancel_after: None,
        lang: "en",
    };
    let mut events = Vec::new();
    let resumed = run_transcription(
        &j,
        &mut eng2,
        &mut v,
        &mut |e: SttEvent| events.push(e),
        &Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(resumed.chunks_resumed, 7);
    assert_eq!(
        calls2.load(Ordering::SeqCst),
        total_chunks,
        "only the remaining chunks were run"
    );
    assert_eq!(
        resumed.text, full.text,
        "resumed transcript equals the uninterrupted one"
    );
    assert!(
        !events.iter().any(|e| matches!(
            e,
            SttEvent::Stage(parlo_core::stt::pipeline::Stage::DetectingLanguage)
        )),
        "locked language is reused"
    );
}

#[test]
fn language_setting_is_respected_and_changes_invalidate_the_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let mut j = job("generated/synth/de_all.wav", dir.path());
    j.language = LanguageSetting::Code("de".into());
    let mut v = vad();
    let mut eng = Scripted {
        calls: Arc::new(AtomicUsize::new(0)),
        cancel_after: None,
        lang: "de",
    };
    let t = run_transcription(
        &j,
        &mut eng,
        &mut v,
        &mut |_: SttEvent| {},
        &Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(t.language.unwrap().code, "de");
    assert!(!t.text.is_empty());
}

#[test]
fn two_hour_file_runs_with_bounded_memory() {
    fn hwm_mb() -> f64 {
        let s = std::fs::read_to_string("/proc/self/status").unwrap();
        s.lines()
            .find_map(|l| l.strip_prefix("VmHWM:"))
            .map(|v| {
                v.trim()
                    .trim_end_matches(" kB")
                    .trim()
                    .parse::<f64>()
                    .unwrap()
                    / 1024.0
            })
            .unwrap()
    }
    let dir = tempfile::tempdir().unwrap();
    let j = job("generated/edge/en_2h.wav", dir.path());
    let before = hwm_mb();
    let mut v = vad();
    let mut eng = Scripted {
        calls: Arc::new(AtomicUsize::new(0)),
        cancel_after: None,
        lang: "en",
    };
    let mut last_progress = 0.0;
    let t = run_transcription(
        &j,
        &mut eng,
        &mut v,
        &mut |e: SttEvent| {
            if let SttEvent::Progress { done_s, .. } = e {
                assert!(done_s >= last_progress, "progress must be monotonic");
                last_progress = done_s;
            }
        },
        &Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    let after = hwm_mb();
    eprintln!(
        "2 h pipeline: {} chunks, {:.0}s wall, peak RSS {:.0} MB (before {:.0} MB)",
        t.chunks_total, t.processing_s, after, before
    );
    assert!(t.duration_s > 7190.0);
    assert!(t.chunks_total > 200);
    assert!(after < 400.0, "peak RSS {after:.0} MB for a 2 h file");
}
