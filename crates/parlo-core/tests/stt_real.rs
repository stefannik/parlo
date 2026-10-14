//! The production STT pipeline with the REAL whisper.cpp engine + Silero VAD (BRIEF §7.2, §14.1): language ID, accuracy on
//! synthetic speech in five languages, no hallucinations on silence/noise/tone/long gaps, and quiet speech (the
//! DECISIONS C-009 regression: FLEURS speakers at −45 dBFS were dropped before normalization).
//! Uses the shipped default `ggml-large-v3-turbo-q8_0` (override: `PARLO_TEST_WHISPER=ggml-small-q8_0.bin` for a fast
//! run — the small model is too weak for the macOS `say` voices, see DECISIONS C-014). Fixtures: scripts/gen-fixtures.sh, models: scripts/fetch-models.sh.
mod common;
use common::{fixture, model, norm_words, wer};
use parlo_core::audio::prepare::prepare_audio;
use parlo_core::audio::wav::{PcmFile, WavWriter};
use parlo_core::stt::pipeline::{run_transcription, LanguageSetting, SttEvent, SttJob, Transcript};
use parlo_core::stt::whisper::{SileroVad, WhisperConfig, WhisperEngine};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

struct Rig {
    engine: WhisperEngine,
    vad: SileroVad,
    dir: tempfile::TempDir,
}

impl Rig {
    fn new() -> Self {
        Self {
            engine: WhisperEngine::load(
                &model(&format!(
                    "whisper/{}",
                    std::env::var("PARLO_TEST_WHISPER")
                        .unwrap_or_else(|_| "ggml-large-v3-turbo-q8_0.bin".into())
                )),
                WhisperConfig::default(),
            )
            .unwrap(),
            vad: SileroVad::load(&model("vad/ggml-silero-v5.1.2.bin"), 300).unwrap(),
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn run(&mut self, input: &std::path::Path, lang: LanguageSetting) -> Transcript {
        let tag = format!(
            "{:x}",
            input.display().to_string().len() ^ (rand_tag() as usize)
        );
        let mut job = SttJob::new(
            input,
            self.dir.path().join(format!("job-{tag}")),
            self.dir.path().join("cache"),
        );
        job.threads = 2;
        job.language = lang;
        run_transcription(
            &job,
            &mut self.engine,
            &mut self.vad,
            &mut |_e: SttEvent| {},
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap()
    }
}

fn rand_tag() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64
}

fn synth_ref(file: &str) -> String {
    let refs = std::fs::read_to_string(fixture("generated/synth/refs.jsonl")).unwrap();
    for l in refs.lines() {
        let v: serde_json::Value = serde_json::from_str(l).unwrap();
        if v["file"] == file {
            return v["text"].as_str().unwrap().to_string();
        }
    }
    panic!("no reference for {file}");
}

#[test]
fn language_id_and_accuracy_on_synthetic_speech_in_five_languages() {
    let mut rig = Rig::new();
    // thresholds: espeak-ng speech is robotic; small-q8 measured 0–15 % WER (spike/stt-bench), turbo is better
    for (lang, max_wer) in [
        ("en", 0.10),
        ("de", 0.30),
        ("fr", 0.30),
        ("es", 0.30),
        ("it", 0.30),
    ] {
        let t = rig.run(
            &fixture(&format!("generated/synth/{lang}_all.wav")),
            LanguageSetting::Auto,
        );
        assert_eq!(
            t.language.as_ref().map(|l| l.code.as_str()),
            Some(lang),
            "language ID for {lang}"
        );
        let w = wer(&synth_ref(&format!("synth/{lang}_all.wav")), &t.text);
        assert!(
            w <= max_wer,
            "{lang}: WER {:.1}% > {:.0}% — {:?}",
            w * 100.0,
            max_wer * 100.0,
            t.text
        );
        assert!(
            t.speech_s > 0.5 * t.duration_s,
            "{lang}: speech {:.1}s of {:.1}s",
            t.speech_s,
            t.duration_s
        );
    }
}

#[test]
fn silence_noise_and_tone_produce_no_text() {
    let mut rig = Rig::new();
    for f in ["silence_60s.wav", "noise_pink_60s.wav", "tone_30s.wav"] {
        let t = rig.run(
            &fixture(&format!("generated/edge/{f}")),
            LanguageSetting::Code("en".into()),
        );
        assert!(t.text.trim().is_empty(), "{f}: hallucinated {:?}", t.text);
    }
}

#[test]
fn long_silences_and_noise_between_utterances_do_not_add_text() {
    let mut rig = Rig::new();
    let t = rig.run(
        &fixture("generated/edge/speech_silence_noise.wav"),
        LanguageSetting::Code("en".into()),
    );
    let reference = synth_ref("synth/en_all.wav");
    let w = wer(&reference, &t.text);
    assert!(w <= 0.15, "WER {:.1}% — {:?}", w * 100.0, t.text);
    assert!(
        norm_words(&t.text).len() as f64 <= 1.2 * norm_words(&reference).len() as f64,
        "extra words from the noise/silence gaps: {:?}",
        t.text
    );
}

#[test]
fn very_quiet_speech_is_still_transcribed() {
    // Regression for DECISIONS C-009: speech at about −40 dBFS used to fall below the energy gate / VAD threshold.
    let mut rig = Rig::new();
    let dir = tempfile::tempdir().unwrap();
    let wav16 = dir.path().join("en16.wav");
    prepare_audio(
        &fixture("generated/synth/en_all.wav"),
        &wav16,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    let pcm = PcmFile::open(&wav16).unwrap();
    let mut samples = Vec::new();
    pcm.read_f32(0, pcm.frames as usize, &mut samples).unwrap();
    let peak = samples.iter().fold(0f32, |m, s| m.max(s.abs()));
    let gain = 0.01 / peak; // peak −40 dBFS
    let quiet = dir.path().join("quiet.wav");
    let mut w = WavWriter::create(&quiet, 1, pcm.sample_rate).unwrap();
    let pcm16: Vec<i16> = samples
        .iter()
        .map(|s| (s * gain * 32767.0).round() as i16)
        .collect();
    w.write_i16(&pcm16).unwrap();
    w.finalize().unwrap();
    let t = rig.run(&quiet, LanguageSetting::Code("en".into()));
    let wr = wer(&synth_ref("synth/en_all.wav"), &t.text);
    assert!(
        wr <= 0.15,
        "quiet speech: WER {:.1}% — {:?}",
        wr * 100.0,
        t.text
    );
}
