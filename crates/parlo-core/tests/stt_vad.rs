//! VAD + chunker against real fixtures (Silero via whisper.cpp).
mod common;
use common::{fixture, model};
use parlo_core::audio::prepare::prepare_audio;
use parlo_core::audio::wav::PcmFile;
use parlo_core::stt::chunker::{build_chunks, ChunkParams};
use parlo_core::stt::vad::{detect_file, Span};
use parlo_core::stt::whisper::SileroVad;
use std::sync::atomic::AtomicBool;

fn pcm_of(rel: &str, dir: &std::path::Path) -> PcmFile {
    let dest = dir.join("p.wav");
    prepare_audio(&fixture(rel), &dest, &AtomicBool::new(false), |_| {}).unwrap();
    PcmFile::open(&dest).unwrap()
}

#[test]
fn vad_finds_speech_between_silence_and_noise_with_correct_units() {
    // fixture layout (gen_fixtures.py): [en_1 speech][20 s silence][en_2 speech][20 s brown noise][en_3 speech]
    let dir = tempfile::tempdir().unwrap();
    let pcm = pcm_of("generated/edge/speech_silence_noise.wav", dir.path());
    let total = pcm.frames as f64 / 16_000.0;
    let mut vad = SileroVad::load(&model("vad/ggml-silero-v5.1.2.bin"), 300).unwrap();
    let spans = detect_file(&pcm, &mut vad, &AtomicBool::new(false), 0.3, |_| {}).unwrap();
    eprintln!("total {total:.1}s spans {spans:?}");
    assert!(spans.len() >= 3, "expected ≥ 3 speech spans, got {spans:?}");
    // the two long non-speech regions (≥ 15 s each) must not be covered
    let covered = |a: f64, b: f64| {
        spans
            .iter()
            .map(|s| (s.end_s.min(b) - s.start_s.max(a)).max(0.0))
            .sum::<f64>()
    };
    let first_end = spans[0].end_s;
    assert!(
        first_end < 8.0,
        "first utterance should end within 8 s, ends at {first_end}"
    );
    assert!(
        covered(first_end + 1.5, first_end + 18.0) < 0.5,
        "silence region was labelled speech: {spans:?}"
    );
    assert!(spans
        .iter()
        .all(|s| s.start_s >= 0.0 && s.end_s <= total + 0.01));
    assert!(
        spans.windows(2).all(|w| w[0].end_s <= w[1].start_s + 1e-6),
        "spans must be sorted and disjoint"
    );
}

#[test]
fn silence_noise_and_tone_files_have_no_speech() {
    let dir = tempfile::tempdir().unwrap();
    let mut vad = SileroVad::load(&model("vad/ggml-silero-v5.1.2.bin"), 300).unwrap();
    for f in ["silence_60s.wav", "noise_pink_60s.wav", "tone_30s.wav"] {
        let pcm = pcm_of(&format!("generated/edge/{f}"), dir.path());
        let spans = detect_file(&pcm, &mut vad, &AtomicBool::new(false), 0.3, |_| {}).unwrap();
        assert!(
            spans.iter().map(|s| s.len()).sum::<f64>() < 1.0,
            "{f}: {spans:?}"
        );
    }
}

#[test]
fn long_speech_is_chunked_to_at_most_30_seconds_cut_in_gaps() {
    let dir = tempfile::tempdir().unwrap();
    let pcm = pcm_of("generated/edge/en_long_10min.wav", dir.path());
    let total = pcm.frames as f64 / 16_000.0;
    let mut vad = SileroVad::load(&model("vad/ggml-silero-v5.1.2.bin"), 300).unwrap();
    let spans = detect_file(&pcm, &mut vad, &AtomicBool::new(false), 0.3, |_| {}).unwrap();
    let mut lowest = |a: f64, b: f64| (a + b) / 2.0;
    let chunks = build_chunks(&spans, total, ChunkParams::default(), &mut lowest);
    assert!(chunks.len() > 15, "{} chunks for {total:.0}s", chunks.len());
    for c in &chunks {
        assert!(c.len() <= 30.0 + 1e-6, "chunk too long: {c:?}");
    }
    // every speech span is fully covered by some chunk
    for s in &spans {
        let covered: f64 = chunks
            .iter()
            .map(|c| (c.end_s.min(s.end_s) - c.start_s.max(s.start_s)).max(0.0))
            .sum();
        assert!(
            covered >= s.len() - 0.05,
            "span {s:?} only covered {covered}"
        );
    }
    assert!(
        chunks
            .windows(2)
            .all(|w: &[Span]| w[0].end_s <= w[1].start_s + 1e-6),
        "chunks overlap"
    );
}
