//! The real PocketTTS engine (patched Candle port) end to end: speak → transcribe with whisper → same words.
mod common;
use common::model;
use parlo_core::audio::resample::StreamResampler;
use parlo_core::paths::ModelPaths;
use parlo_core::stt::engine::{SttEngine, TranscribeOpts};
use parlo_core::stt::whisper::{WhisperConfig, WhisperEngine};
use parlo_core::tts::pocket::{PocketEngine, VOICES};
use parlo_core::tts::{Quality, TtsEngine};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

fn engine() -> PocketEngine {
    let root = model("pockettts/english/model.safetensors"); // asserts the models exist with a helpful message
    PocketEngine::load(&ModelPaths::new(root.ancestors().nth(3).unwrap())).unwrap()
}

fn speak(e: &mut PocketEngine, text: &str, voice: &str, q: Quality) -> Vec<f32> {
    let mut audio = Vec::new();
    e.synthesize(
        text,
        voice,
        q,
        &mut |s: &[f32]| {
            audio.extend_from_slice(s);
            Ok(())
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    audio
}

fn transcribe_24k(audio: &[f32]) -> String {
    let mut rs = StreamResampler::new(24_000, 16_000).unwrap();
    let mut out = Vec::new();
    rs.process(audio, &mut out).unwrap();
    rs.finish(&mut out).unwrap();
    let mut w = WhisperEngine::load(
        &model("whisper/ggml-small-q8_0.bin"),
        WhisperConfig::default(),
    )
    .unwrap();
    let t = w
        .transcribe(
            &out,
            &TranscribeOpts {
                language: Some("en".into()),
                initial_prompt: None,
                threads: 4,
            },
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
    t.segments
        .iter()
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect()
}

#[test]
fn all_voices_are_on_disk_and_listed_with_their_language() {
    let e = engine();
    let got: Vec<(String, String)> = e.voices().into_iter().map(|v| (v.id, v.language)).collect();
    let want: Vec<(String, String)> = VOICES
        .iter()
        .map(|(id, _, lang)| (id.to_string(), lang.to_string()))
        .collect();
    assert_eq!(got, want);
    assert_eq!(got.len(), 22);
    assert_eq!(got.iter().filter(|(_, l)| l == "en").count(), 21);
    assert_eq!(e.sample_rate(), 24_000);
    assert!(e
        .voices()
        .iter()
        .any(|v| v.id == "george" && v.label == "George"));
}

#[test]
fn speech_round_trips_through_whisper_for_two_voices_and_both_qualities() {
    let mut e = engine();
    let text =
        "The quick brown fox jumps over the lazy dog. Please call me back after the meeting.";
    for (voice, q) in [("george", Quality::Standard), ("alba", Quality::High)] {
        let audio = speak(&mut e, text, voice, q);
        let secs = audio.len() as f64 / 24_000.0;
        assert!(
            (3.5..9.0).contains(&secs),
            "{voice}: {secs:.1}s of audio for 15 words"
        );
        assert!(audio.iter().all(|x| x.is_finite()), "no NaNs");
        let peak = audio.iter().fold(0f32, |m, x| m.max(x.abs()));
        assert!((0.05..=1.0).contains(&peak), "{voice}: peak {peak}");
        let got = words(&transcribe_24k(&audio));
        let want = words(text);
        let hits = want.iter().filter(|w| got.contains(w)).count();
        assert!(
            hits as f64 >= 0.85 * want.len() as f64,
            "{voice}/{q:?}: heard {got:?}"
        );
    }
}

#[test]
fn abort_stops_synthesis_quickly() {
    let mut e = engine();
    let abort = AtomicBool::new(false);
    let t = std::time::Instant::now();
    let mut n = 0usize;
    let r = e.synthesize(
        &"This is a rather long sentence that keeps going. ".repeat(10),
        "george",
        Quality::Standard,
        &mut |s: &[f32]| {
            n += s.len();
            if n > 24_000 {
                abort.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            Ok(())
        },
        &abort,
    );
    assert!(matches!(r, Err(parlo_core::Error::Cancelled)), "{r:?}");
    assert!(
        t.elapsed().as_secs_f64() < 15.0,
        "abort took {:.1}s",
        t.elapsed().as_secs_f64()
    );
}

#[test]
fn spanish_voice_speaks_spanish_and_switching_languages_keeps_both_working() {
    let mut e = engine();
    let es = speak(
        &mut e,
        "Hola, esta es una prueba rápida de la nueva voz en español.",
        "lola",
        Quality::Standard,
    );
    let secs = es.len() as f64 / 24_000.0;
    assert!((2.5..9.0).contains(&secs), "{secs:.1}s");
    // back to English (the Spanish model replaced the English one in memory; it must reload transparently)
    let en = speak(
        &mut e,
        "This is a short English sentence.",
        "george",
        Quality::Standard,
    );
    assert!(en.len() as f64 / 24_000.0 > 1.5);
    let mut w = WhisperEngine::load(
        &model("whisper/ggml-small-q8_0.bin"),
        WhisperConfig::default(),
    )
    .unwrap();
    let mut rs = StreamResampler::new(24_000, 16_000).unwrap();
    let mut out = Vec::new();
    rs.process(&es, &mut out).unwrap();
    rs.finish(&mut out).unwrap();
    let t = w
        .transcribe(
            &out,
            &TranscribeOpts {
                language: Some("es".into()),
                initial_prompt: None,
                threads: 4,
            },
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
    let heard = words(
        &t.segments
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" "),
    );
    let want = words("Hola, esta es una prueba rápida de la nueva voz en español.");
    let hits = want.iter().filter(|w| heard.contains(w)).count();
    assert!(hits as f64 >= 0.8 * want.len() as f64, "heard {heard:?}");
}

#[test]
fn text_preparation_matches_pocket_tts_3x() {
    use pocket_tts::tts_model::{prepare_text_prompt_cfg, TextPrep};
    let cfg = TextPrep::default();
    // terminal punctuation: weak marks become a period, closers are kept, bare text gets one
    assert_eq!(
        prepare_text_prompt_cfg("hello there,", &cfg).0,
        "Hello there."
    );
    assert_eq!(
        prepare_text_prompt_cfg("He said \"go\"", &cfg).0,
        "He said \"go\"."
    );
    assert_eq!(prepare_text_prompt_cfg("Really?\"", &cfg).0, "Really?\"");
    assert_eq!(prepare_text_prompt_cfg("Wait -", &cfg).0, "Wait.");
    // few words ⇒ more frames after EOS
    assert_eq!(prepare_text_prompt_cfg("Hi there.", &cfg).1, 3);
    assert_eq!(
        prepare_text_prompt_cfg("This one has more than four words.", &cfg).1,
        1
    );
}
