//! TTS long-text pipeline (BRIEF §9.3, §14.1) with a deterministic tone engine standing in for PocketTTS:
//! segments + silences, progress, segment_ready events, content-addressed cache, crash resume, bounded RAM on 50 k words.
use parlo_core::audio::wav::PcmFile;
use parlo_core::error::{Error, Result};
use parlo_core::text::segment::{PAUSE_HEADING_MS, PAUSE_PARAGRAPH_MS};
use parlo_core::text::Block;
use parlo_core::tts::{run_tts, Quality, TtsEngine, TtsEvent, TtsJob, VoiceInfo};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

/// `ms_per_char` of audio per character: a 220 Hz tone whose phase depends only on the text (so reruns are bit-identical).
struct ToneEngine {
    ms_per_char: f64,
    calls: Arc<AtomicUsize>,
    /// fail (simulated crash) when this many calls have been made in total
    fail_on_call: Option<usize>,
    revision: &'static str,
}
impl ToneEngine {
    fn new(ms_per_char: f64) -> Self {
        Self {
            ms_per_char,
            calls: Arc::new(AtomicUsize::new(0)),
            fail_on_call: None,
            revision: "test-rev-1",
        }
    }
}
impl TtsEngine for ToneEngine {
    fn id(&self) -> &str {
        "tone-test"
    }
    fn model_revision(&self) -> &str {
        self.revision
    }
    fn sample_rate(&self) -> u32 {
        24_000
    }
    fn voices(&self) -> Vec<VoiceInfo> {
        vec![
            VoiceInfo {
                id: "george".into(),
                label: "George".into(),
            },
            VoiceInfo {
                id: "alba".into(),
                label: "Alba".into(),
            },
        ]
    }
    fn synthesize(
        &mut self,
        text: &str,
        voice: &str,
        quality: Quality,
        sink: &mut dyn FnMut(&[f32]) -> Result<()>,
        abort: &AtomicBool,
    ) -> Result<()> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        let frames = (text.chars().count() as f64 * self.ms_per_char / 1000.0 * 24_000.0) as usize;
        let seed = text.bytes().fold(
            voice.len() as u32 + quality.lsd_decode_steps() as u32,
            |a, b| a.wrapping_mul(31).wrapping_add(b as u32),
        ) % 97;
        let mut done = 0;
        while done < frames {
            if abort.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            let k = 4000.min(frames - done);
            let block: Vec<f32> = (0..k)
                .map(|i| {
                    (((done + i) as f32) * (220.0 + seed as f32) * std::f32::consts::TAU / 24_000.0)
                        .sin()
                        * 0.3
                })
                .collect();
            sink(&block)?;
            done += k;
            if let Some(f) = self.fail_on_call {
                if n >= f && done > frames / 2 {
                    return Err(Error::Other("simulated crash mid-chunk".into()));
                }
            }
        }
        Ok(())
    }
}

fn job(dir: &std::path::Path, blocks: Vec<Block>) -> TtsJob {
    TtsJob {
        blocks,
        voice: "george".into(),
        quality: Quality::Standard,
        outputs_dir: dir.join("outputs"),
        job_dir: dir.join("jobs/j1"),
    }
}

fn para(chars: usize, tag: &str) -> Block {
    let mut s = format!("{tag} ");
    while s.len() < chars {
        s.push_str("word ");
    }
    s.truncate(chars - 1);
    s.push('.');
    Block::para(s)
}

fn all_samples(paths: &[std::path::PathBuf]) -> Vec<i16> {
    let mut v = Vec::new();
    for p in paths {
        v.extend(
            hound::WavReader::open(p)
                .unwrap()
                .samples::<i16>()
                .map(|s| s.unwrap()),
        );
    }
    v
}

#[test]
fn segments_silences_and_events_are_correct() {
    let dir = tempfile::tempdir().unwrap();
    // 40 ms/char: a 700-char paragraph = 28 s of audio -> the first segment (target 25 s) closes after paragraph 1
    let blocks = vec![
        Block::heading("Chapter one"),
        para(700, "A"),
        para(700, "B"),
        para(700, "C"),
        Block::heading("Chapter two"),
        para(300, "D"),
    ];
    let mut eng = ToneEngine::new(40.0);
    let mut events = Vec::new();
    let out = run_tts(
        &job(dir.path(), blocks),
        &mut eng,
        &mut |e: TtsEvent| events.push(e),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(!out.cache_hit);
    // first segment is ready early (heading + paragraph A), later ones follow
    let ready: Vec<_> = events
        .iter()
        .filter_map(|e| {
            if let TtsEvent::SegmentReady {
                index, duration_ms, ..
            } = e
            {
                Some((*index, *duration_ms))
            } else {
                None
            }
        })
        .collect();
    assert!(ready.len() >= 2, "{ready:?}");
    assert_eq!(
        ready.iter().map(|r| r.0).collect::<Vec<_>>(),
        (0..ready.len()).collect::<Vec<_>>()
    );
    assert!(
        ready[0].1 >= 25_000,
        "first segment closes once ≥ 25 s: {ready:?}"
    );
    // total duration = text + generated silences
    let chars: usize = [
        "Chapter one",
        &"x".repeat(700),
        &"x".repeat(700),
        &"x".repeat(700),
        "Chapter two",
        &"x".repeat(300),
    ]
    .iter()
    .map(|s| s.len())
    .sum();
    let expect_ms =
        (chars as f64 * 40.0) as u64 + 2 * PAUSE_HEADING_MS as u64 + 4 * PAUSE_PARAGRAPH_MS as u64;
    let got: u64 = out.meta.segments.iter().map(|s| s.duration_ms).sum();
    assert!(
        (got as i64 - expect_ms as i64).abs() < 50,
        "duration {got} ms vs expected {expect_ms} ms"
    );
    assert_eq!(out.meta.duration_ms, got);
    // progress is monotonic and ends at 100 %
    let prog: Vec<usize> = events
        .iter()
        .filter_map(|e| {
            if let TtsEvent::Progress { done_chars, .. } = e {
                Some(*done_chars)
            } else {
                None
            }
        })
        .collect();
    assert!(prog.windows(2).all(|w| w[0] <= w[1]));
    assert_eq!(*prog.last().unwrap(), chars);
    assert!(matches!(events.last().unwrap(), TtsEvent::Done { .. }));
    // all segments are valid 24 kHz mono WAVs
    for p in &out.segments {
        let f = PcmFile::open(p).unwrap();
        assert_eq!((f.sample_rate, f.channels), (24_000, 1));
    }
    // the job checkpoint directory is cleaned up on success
    assert!(!dir.path().join("jobs/j1").exists());
}

#[test]
fn identical_request_is_an_instant_cache_hit_and_any_change_misses() {
    let dir = tempfile::tempdir().unwrap();
    let blocks = vec![para(400, "A"), para(400, "B")];
    let mut eng = ToneEngine::new(20.0);
    let calls = eng.calls.clone();
    let first = run_tts(
        &job(dir.path(), blocks.clone()),
        &mut eng,
        &mut |_: TtsEvent| {},
        &AtomicBool::new(false),
    )
    .unwrap();
    let n = calls.load(Ordering::SeqCst);
    assert!(n >= 2);
    let t = std::time::Instant::now();
    let mut events = Vec::new();
    let again = run_tts(
        &job(dir.path(), blocks.clone()),
        &mut eng,
        &mut |e: TtsEvent| events.push(e),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(again.cache_hit && again.output_id == first.output_id);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        n,
        "engine not called on a cache hit"
    );
    assert!(t.elapsed().as_millis() < 100, "cache hit must be instant");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, TtsEvent::SegmentReady { .. }))
            && matches!(events.last(), Some(TtsEvent::Done { .. }))
    );
    // any change of voice / quality / text / model revision is a different output
    let mut j = job(dir.path(), blocks.clone());
    j.voice = "alba".into();
    assert_ne!(
        run_tts(&j, &mut eng, &mut |_: TtsEvent| {}, &AtomicBool::new(false))
            .unwrap()
            .output_id,
        first.output_id
    );
    let mut j = job(dir.path(), blocks.clone());
    j.quality = Quality::Fast;
    assert_ne!(
        run_tts(&j, &mut eng, &mut |_: TtsEvent| {}, &AtomicBool::new(false))
            .unwrap()
            .output_id,
        first.output_id
    );
    let mut j2 = job(dir.path(), vec![para(400, "A"), para(400, "Z")]);
    j2.job_dir = dir.path().join("jobs/j2");
    assert_ne!(
        run_tts(
            &j2,
            &mut eng,
            &mut |_: TtsEvent| {},
            &AtomicBool::new(false)
        )
        .unwrap()
        .output_id,
        first.output_id
    );
    let mut eng2 = ToneEngine::new(20.0);
    eng2.revision = "test-rev-2";
    assert_ne!(
        run_tts(
            &job(dir.path(), blocks),
            &mut eng2,
            &mut |_: TtsEvent| {},
            &AtomicBool::new(false)
        )
        .unwrap()
        .output_id,
        first.output_id
    );
}

#[test]
fn crash_mid_chunk_then_resume_yields_the_identical_audio() {
    // reference: uninterrupted
    let blocks: Vec<Block> = std::iter::once(Block::heading("Title"))
        .chain((0..12).map(|i| para(500, &format!("P{i}"))))
        .collect();
    let ref_dir = tempfile::tempdir().unwrap();
    let mut eng = ToneEngine::new(30.0);
    let full = run_tts(
        &job(ref_dir.path(), blocks.clone()),
        &mut eng,
        &mut |_: TtsEvent| {},
        &AtomicBool::new(false),
    )
    .unwrap();

    // crash during the 6th synthesize call
    let dir = tempfile::tempdir().unwrap();
    let mut eng1 = ToneEngine::new(30.0);
    eng1.fail_on_call = Some(6);
    let r = run_tts(
        &job(dir.path(), blocks.clone()),
        &mut eng1,
        &mut |_: TtsEvent| {},
        &AtomicBool::new(false),
    );
    assert!(matches!(r, Err(Error::Other(m)) if m.contains("simulated crash")));
    assert!(
        dir.path().join("jobs/j1/manifest.json").exists(),
        "checkpoint survives"
    );

    // "relaunch": a fresh engine resumes; only the failed chunk and the rest are synthesized
    let mut eng2 = ToneEngine::new(30.0);
    let calls2 = eng2.calls.clone();
    let mut events = Vec::new();
    let resumed = run_tts(
        &job(dir.path(), blocks.clone()),
        &mut eng2,
        &mut |e: TtsEvent| events.push(e),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        calls2.load(Ordering::SeqCst),
        blocks.len() - 5,
        "chunks 1–5 were durable; resume starts at chunk 6"
    );
    assert_eq!(
        all_samples(&resumed.segments),
        all_samples(&full.segments),
        "resumed audio is bit-identical to the uninterrupted run"
    );
    assert_eq!(resumed.meta.duration_ms, full.meta.duration_ms);
    // the resumed run replays finished segments to the sink so a UI can rebuild its playlist
    assert!(events
        .iter()
        .any(|e| matches!(e, TtsEvent::SegmentReady { index: 0, .. })));
}

#[test]
fn cancel_stops_promptly_and_can_be_resumed() {
    let dir = tempfile::tempdir().unwrap();
    let blocks: Vec<Block> = (0..8).map(|i| para(600, &format!("P{i}"))).collect();
    let cancel = AtomicBool::new(false);
    let mut eng = ToneEngine::new(30.0);
    let calls = eng.calls.clone();
    let mut n_progress = 0;
    let r = run_tts(
        &job(dir.path(), blocks.clone()),
        &mut eng,
        &mut |e: TtsEvent| {
            if matches!(e, TtsEvent::Progress { .. }) {
                n_progress += 1;
                if n_progress == 4 {
                    cancel.store(true, Ordering::SeqCst);
                }
            }
        },
        &cancel,
    );
    assert!(matches!(r, Err(Error::Cancelled)));
    assert!(calls.load(Ordering::SeqCst) <= 4);
    let mut eng2 = ToneEngine::new(30.0);
    let out = run_tts(
        &job(dir.path(), blocks),
        &mut eng2,
        &mut |_: TtsEvent| {},
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(out.meta.complete);
}

#[test]
fn unknown_voice_and_empty_text_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let mut eng = ToneEngine::new(10.0);
    let mut j = job(dir.path(), vec![para(100, "A")]);
    j.voice = "nobody".into();
    assert!(
        matches!(run_tts(&j, &mut eng, &mut |_: TtsEvent| {}, &AtomicBool::new(false)), Err(Error::Other(m)) if m.contains("not available"))
    );
    assert!(
        matches!(run_tts(&job(dir.path(), vec![Block::para("   ")]), &mut eng, &mut |_: TtsEvent| {}, &AtomicBool::new(false)), Err(Error::Other(m)) if m.contains("no text"))
    );
}

#[test]
fn fifty_thousand_words_complete_with_bounded_memory_monotonic_progress_and_resume() {
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
    // 50 000 words ≈ 300 000 characters in 1 000 paragraphs of ~300 chars
    let words = ["alpha", "bravo", "charlie", "delta", "echo", "foxtrot"];
    let blocks: Vec<Block> = (0..1000)
        .map(|p| {
            let mut s = String::new();
            for w in 0..50 {
                s.push_str(words[(p + w) % words.len()]);
                s.push(' ');
            }
            s.truncate(s.trim_end().len());
            s.push('.');
            Block::para(s)
        })
        .collect();
    let total_words: usize = blocks
        .iter()
        .map(|b| b.text.split_whitespace().count())
        .sum();
    assert!(total_words >= 50_000);
    let before = hwm_mb();
    let mut eng = ToneEngine::new(60.0);
    let calls = eng.calls.clone();
    let mut last = 0usize;
    let mut segs = 0usize;
    let cancel = AtomicBool::new(false);
    // stop halfway ("quit mid-way"), then resume
    let r = run_tts(
        &job(dir.path(), blocks.clone()),
        &mut eng,
        &mut |e: TtsEvent| match e {
            TtsEvent::Progress {
                done_chars,
                total_chars,
                ..
            } => {
                assert!(done_chars >= last, "progress must be monotonic");
                last = done_chars;
                if done_chars * 2 > total_chars {
                    cancel.store(true, Ordering::SeqCst);
                }
            }
            TtsEvent::SegmentReady { .. } => segs += 1,
            _ => {}
        },
        &cancel,
    );
    assert!(matches!(r, Err(Error::Cancelled)));
    let first_run_calls = calls.load(Ordering::SeqCst);
    let mut segs2 = 0usize;
    let mut eng2 = ToneEngine::new(60.0);
    let calls2 = eng2.calls.clone();
    let out = run_tts(
        &job(dir.path(), blocks),
        &mut eng2,
        &mut |e: TtsEvent| {
            if matches!(e, TtsEvent::SegmentReady { .. }) {
                segs2 += 1;
            }
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        first_run_calls + calls2.load(Ordering::SeqCst),
        1000 + 0,
        "every chunk synthesized exactly once across both runs (cancel is checked between chunks)"
    );
    let after = hwm_mb();
    eprintln!(
        "50k words: {} segments, {:.1} h of audio, peak RSS {:.0} MB (before {:.0} MB)",
        out.meta.segments.len(),
        out.meta.duration_ms as f64 / 3.6e6,
        after,
        before
    );
    assert!(out.meta.segments.len() >= 6 && segs + segs2 >= out.meta.segments.len());
    assert!(
        out.meta.duration_ms > 4 * 3_600_000,
        "a ~50k-word book is hours of audio"
    );
    assert!(
        after - before < 300.0,
        "RSS grew by {:.0} MB",
        after - before
    );
}
