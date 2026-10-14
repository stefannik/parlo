//! The real PocketTTS engine (patched Candle port) end to end: speak → transcribe with whisper → same words.
mod common;
use common::model;
use parlo_core::audio::resample::StreamResampler;
use parlo_core::paths::ModelPaths;
use parlo_core::stt::engine::{SttEngine, TranscribeOpts};
use parlo_core::stt::whisper::{WhisperConfig, WhisperEngine};
use parlo_core::tts::pocket::{PocketEngine, VOICE_IDS};
use parlo_core::tts::{Quality, TtsEngine};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

fn engine() -> PocketEngine {
    let root = model("pockettts/hf"); // asserts the models exist with a helpful message
    PocketEngine::load(&ModelPaths::new(root.parent().unwrap().parent().unwrap())).unwrap()
}

fn speak(e: &mut PocketEngine, text: &str, voice: &str, q: Quality) -> Vec<f32> {
    let mut audio = Vec::new();
    e.synthesize(text, voice, q, &mut |s: &[f32]| { audio.extend_from_slice(s); Ok(()) }, &AtomicBool::new(false)).unwrap();
    audio
}

fn transcribe_24k(audio: &[f32]) -> String {
    let mut rs = StreamResampler::new(24_000, 16_000).unwrap();
    let mut out = Vec::new();
    rs.process(audio, &mut out).unwrap();
    rs.finish(&mut out).unwrap();
    let mut w = WhisperEngine::load(&model("whisper/ggml-small-q8_0.bin"), WhisperConfig::default()).unwrap();
    let t = w.transcribe(&out, &TranscribeOpts { language: Some("en".into()), initial_prompt: None, threads: 4 }, &Arc::new(AtomicBool::new(false))).unwrap();
    t.segments.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join(" ")
}

fn words(s: &str) -> Vec<String> {
    s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(String::from).collect()
}

#[test]
fn all_ten_voices_are_on_disk_and_listed() {
    let e = engine();
    let ids: Vec<String> = e.voices().into_iter().map(|v| v.id).collect();
    assert_eq!(ids, VOICE_IDS.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    assert_eq!(e.sample_rate(), 24_000);
    assert!(e.voices().iter().any(|v| v.id == "george" && v.label == "George"));
}

#[test]
fn speech_round_trips_through_whisper_for_two_voices_and_both_qualities() {
    let mut e = engine();
    let text = "The quick brown fox jumps over the lazy dog. Please call me back after the meeting.";
    for (voice, q) in [("george", Quality::Standard), ("alba", Quality::Fast)] {
        let audio = speak(&mut e, text, voice, q);
        let secs = audio.len() as f64 / 24_000.0;
        assert!((3.5..9.0).contains(&secs), "{voice}: {secs:.1}s of audio for 15 words");
        assert!(audio.iter().all(|x| x.is_finite()), "no NaNs");
        let peak = audio.iter().fold(0f32, |m, x| m.max(x.abs()));
        assert!((0.05..=1.0).contains(&peak), "{voice}: peak {peak}");
        let got = words(&transcribe_24k(&audio));
        let want = words(text);
        let hits = want.iter().filter(|w| got.contains(w)).count();
        assert!(hits as f64 >= 0.85 * want.len() as f64, "{voice}/{q:?}: heard {got:?}");
    }
}

#[test]
fn abort_stops_synthesis_quickly() {
    let mut e = engine();
    let abort = AtomicBool::new(false);
    let t = std::time::Instant::now();
    let mut n = 0usize;
    let r = e.synthesize(&"This is a rather long sentence that keeps going. ".repeat(10), "george", Quality::Standard, &mut |s: &[f32]| { n += s.len(); if n > 24_000 { abort.store(true, std::sync::atomic::Ordering::SeqCst); } Ok(()) }, &abort);
    assert!(matches!(r, Err(parlo_core::Error::Cancelled)), "{r:?}");
    assert!(t.elapsed().as_secs_f64() < 15.0, "abort took {:.1}s", t.elapsed().as_secs_f64());
}
