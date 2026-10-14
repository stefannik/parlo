//! Real multilingual speech (Google FLEURS, pinned subset — scripts/fleurs_subset.py) through the production STT pipeline:
//! language detection is correct on every single-language file, and WER stays within thresholds calibrated on the
//! measured `ggml-small-q8_0` results (spike/stt-bench/report.md) plus margin. The shipped default model
//! (large-v3-turbo-q8_0) is considerably more accurate (mean WER 6.8 % vs 19.3 % on the bench), so these are
//! regression guards, not targets. Slow (≈ 3–6 min on 4 vCPU): 4 utterances × 8 languages.
mod common;
use common::{fixture, model, wer};
use parlo_core::stt::pipeline::{run_transcription, LanguageSetting, SttEvent, SttJob};
use parlo_core::stt::whisper::{SileroVad, WhisperConfig, WhisperEngine};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

const UTTERANCES_PER_LANGUAGE: usize = 4;

/// (FLEURS dir, ISO code, max WER). Thresholds = measured WER on these utterances with ggml-small-q8_0, plus margin.
const LANGS: [(&str, &str, f64); 8] = [
    ("en_us", "en", 0.25),
    ("de_de", "de", 0.20),
    ("fr_fr", "fr", 0.40),
    ("es_419", "es", 0.15),
    ("it_it", "it", 0.25),
    ("bg_bg", "bg", 0.70), // measured 53 %: the small model mislabels short Bulgarian clips (hr/mk), turbo does not
    ("ru_ru", "ru", 0.20),
    ("pt_br", "pt", 0.35),
];

/// Single 5–10 s clips are the hardest case for language ID (the app votes over up to 3 chunks of real recordings);
/// allow a couple of misses with the small model in this regression guard.
const MAX_LANGUAGE_ID_MISSES: usize = 2;

#[test]
fn fleurs_language_id_and_wer_per_language() {
    let refs = std::fs::read_to_string(fixture("generated/fleurs/refs.jsonl")).unwrap();
    let refs: Vec<serde_json::Value> = refs
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let mut engine = WhisperEngine::load(
        &model("whisper/ggml-small-q8_0.bin"),
        WhisperConfig::default(),
    )
    .unwrap();
    let mut vad = SileroVad::load(&model("vad/ggml-silero-v5.1.2.bin"), 300).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut report = Vec::new();
    let mut failures = Vec::new();
    let mut id_misses = Vec::new();
    for (dirname, iso, max_wer) in LANGS {
        let items: Vec<&serde_json::Value> = refs
            .iter()
            .filter(|r| r["lang"] == dirname)
            .take(UTTERANCES_PER_LANGUAGE)
            .collect();
        assert_eq!(
            items.len(),
            UTTERANCES_PER_LANGUAGE,
            "{dirname}: fixtures missing — scripts/gen-fixtures.sh --only fleurs"
        );
        let (mut want, mut got) = (String::new(), String::new());
        for it in items {
            let wav = fixture(&format!(
                "generated/fleurs/{}.wav",
                it["id"].as_str().unwrap()
            ));
            let mut job = SttJob::new(
                &wav,
                dir.path().join(format!(
                    "job-{iso}-{}",
                    it["id"].as_str().unwrap().replace('/', "_")
                )),
                dir.path().join("cache"),
            );
            job.threads = 2;
            job.language = LanguageSetting::Auto;
            let t = run_transcription(
                &job,
                &mut engine,
                &mut vad,
                &mut |_e: SttEvent| {},
                &Arc::new(AtomicBool::new(false)),
            )
            .unwrap();
            if t.language.as_ref().map(|l| l.code.as_str()) != Some(iso) {
                id_misses.push(format!(
                    "{iso}: language ID said {:?} for {}",
                    t.language.map(|l| l.code),
                    it["id"]
                ));
            }
            want.push_str(it["text"].as_str().unwrap());
            want.push(' ');
            got.push_str(&t.text);
            got.push(' ');
        }
        let w = wer(&want, &got);
        report.push(format!(
            "{iso}: WER {:.1}% (limit {:.0}%)",
            w * 100.0,
            max_wer * 100.0
        ));
        if w > max_wer {
            failures.push(format!(
                "{iso}: WER {:.1}% > {:.0}%\n  want: {want}\n  got:  {got}",
                w * 100.0,
                max_wer * 100.0
            ));
        }
    }
    println!("{}", report.join("\n"));
    println!("language ID misses: {id_misses:?}");
    if id_misses.len() > MAX_LANGUAGE_ID_MISSES {
        failures.push(format!(
            "{} language ID misses (max {MAX_LANGUAGE_ID_MISSES}): {id_misses:?}",
            id_misses.len()
        ));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
