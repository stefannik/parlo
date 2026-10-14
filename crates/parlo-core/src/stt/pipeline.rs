//! The transcription job (BRIEF §7.2): prepare → VAD → chunk → language lock → transcribe chunk by chunk →
//! post-filter → paragraphs, checkpointing every chunk. Memory is bounded: audio is read chunk-wise from a 16 kHz
//! PCM file on disk.

use super::assemble::{Assembler, Paragraph, Seg};
use super::checkpoint::{Checkpoint, ChunkRecord, JobMeta};
use super::chunker::{build_chunks, ChunkParams};
use super::engine::{LangGuess, SttEngine, TranscribeOpts};
use super::filters::{
    analyze, collapse_repeats, normalize_peak, should_drop_segment, should_transcribe,
};
use super::vad::{detect_file, SpeechDetector};
use crate::audio::prepare::{prepare_audio_with, STT_RATE};
use crate::audio::wav::PcmFile;
use crate::error::{Error, Result};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Clone, PartialEq)]
pub enum LanguageSetting {
    Auto,
    Code(String),
}

#[derive(Debug, Clone)]
pub struct SttJob {
    pub input: PathBuf,
    /// identity of the input for checkpoint validity (e.g. `path|size|mtime` or a recording id)
    pub input_key: String,
    pub job_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub language: LanguageSetting,
    pub threads: usize,
    /// feed the previous chunk's tail to the decoder as context (off by default; see DECISIONS)
    pub carry_prompt: bool,
    pub chunk: ChunkParams,
    /// platform decoder tried when the portable one can't read the file (macOS: AVFoundation)
    pub fallback_decoder: Option<std::sync::Arc<dyn crate::platform::FallbackDecoder>>,
}

impl SttJob {
    pub fn new(
        input: impl Into<PathBuf>,
        job_dir: impl Into<PathBuf>,
        cache_dir: impl Into<PathBuf>,
    ) -> Self {
        let input = input.into();
        let key = input_key_for(&input);
        Self {
            input,
            input_key: key,
            job_dir: job_dir.into(),
            cache_dir: cache_dir.into(),
            language: LanguageSetting::Auto,
            threads: 4,
            carry_prompt: false,
            chunk: ChunkParams::default(),
            fallback_decoder: None,
        }
    }
}

pub fn input_key_for(path: &std::path::Path) -> String {
    let md = std::fs::metadata(path).ok();
    let size = md.as_ref().map(|m| m.len()).unwrap_or(0);
    let mtime = md
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}|{}|{}", path.display(), size, mtime)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Preparing,
    DetectingSpeech,
    DetectingLanguage,
    Transcribing,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SttEvent {
    Stage(Stage),
    /// progress of the current pre-processing stage, 0..=1
    StageProgress(f64),
    Language(LangGuess),
    Progress {
        done_s: f64,
        total_s: f64,
        eta_ms: Option<u64>,
    },
    Paragraph(Paragraph),
}

pub trait SttSink {
    fn event(&mut self, e: SttEvent);
}
impl<F: FnMut(SttEvent)> SttSink for F {
    fn event(&mut self, e: SttEvent) {
        self(e)
    }
}

#[derive(Debug, Clone)]
pub struct Transcript {
    pub paragraphs: Vec<Paragraph>,
    pub segments: Vec<Seg>,
    pub text: String,
    pub language: Option<LangGuess>,
    pub duration_s: f64,
    pub speech_s: f64,
    pub word_count: usize,
    pub processing_s: f64,
    pub chunks_total: usize,
    pub chunks_resumed: usize,
}

fn vote_language(guesses: &[LangGuess]) -> Option<LangGuess> {
    let mut score: HashMap<&str, (u32, f32)> = HashMap::new();
    for g in guesses {
        let e = score.entry(g.code.as_str()).or_insert((0, 0.0));
        e.0 += 1;
        e.1 += g.prob;
    }
    // majority vote; ties broken by summed probability
    let (code, (n, sum)) = score.into_iter().max_by(|a, b| {
        a.1 .0.cmp(&b.1 .0).then(
            a.1 .1
                .partial_cmp(&b.1 .1)
                .unwrap_or(std::cmp::Ordering::Equal),
        )
    })?;
    Some(LangGuess {
        code: code.to_string(),
        prob: sum / n as f32,
    })
}

pub fn run_transcription(
    job: &SttJob,
    engine: &mut dyn SttEngine,
    vad: &mut dyn SpeechDetector,
    sink: &mut dyn SttSink,
    cancel: &Arc<AtomicBool>,
) -> Result<Transcript> {
    let t_start = Instant::now();
    let cp = Checkpoint::new(&job.job_dir);
    let lang_setting = match &job.language {
        LanguageSetting::Auto => "auto".to_string(),
        LanguageSetting::Code(c) => c.clone(),
    };
    // a checkpoint is only valid for the same input, model and language setting
    let mut meta = match cp.load_meta() {
        Some(m)
            if m.input_key == job.input_key
                && m.model == engine.id()
                && m.language_setting == lang_setting =>
        {
            m
        }
        Some(_) => {
            cp.clear();
            fresh_meta(job, engine, &lang_setting)
        }
        None => fresh_meta(job, engine, &lang_setting),
    };
    let done_chunks: HashMap<usize, ChunkRecord> =
        cp.load_chunks().into_iter().map(|c| (c.index, c)).collect();

    // 1) prepare
    sink.event(SttEvent::Stage(Stage::Preparing));
    std::fs::create_dir_all(&job.cache_dir).map_err(|e| Error::io(&job.cache_dir, e))?;
    let key_hash = fnv1a(job.input_key.as_bytes());
    let pcm_path = job.cache_dir.join(format!("stt-{key_hash:016x}.wav"));
    let reuse = pcm_path.exists()
        && PcmFile::open(&pcm_path)
            .map(|f| f.frames > 0)
            .unwrap_or(false);
    if !reuse {
        prepare_audio_with(
            &job.input,
            &pcm_path,
            cancel,
            |p| sink.event(SttEvent::StageProgress(p)),
            job.fallback_decoder.as_deref(),
        )?;
    }
    let pcm = PcmFile::open(&pcm_path)?;
    debug_assert_eq!(pcm.sample_rate, STT_RATE);
    let total_s = pcm.frames as f64 / STT_RATE as f64;

    // 2) VAD → chunks
    sink.event(SttEvent::Stage(Stage::DetectingSpeech));
    let spans = detect_file(&pcm, vad, cancel, 0.3, |p| {
        sink.event(SttEvent::StageProgress(p))
    })?;
    let speech_s: f64 = spans.iter().map(|s| s.len()).sum();
    let mut scratch = Vec::new();
    let mut lowest = |a: f64, b: f64| -> f64 {
        if pcm
            .read_f32(
                (a * STT_RATE as f64) as u64,
                ((b - a) * STT_RATE as f64) as usize,
                &mut scratch,
            )
            .is_err()
            || scratch.is_empty()
        {
            return b;
        }
        let hop = (STT_RATE as usize / 50).max(1); // 20 ms
        let (mut best_i, mut best_e) = (0usize, f32::MAX);
        for (i, f) in scratch.chunks(hop).enumerate() {
            let e: f32 = f.iter().map(|x| x * x).sum::<f32>() / f.len() as f32;
            if e < best_e {
                best_e = e;
                best_i = i;
            }
        }
        a + (best_i * hop + hop / 2) as f64 / STT_RATE as f64
    };
    let chunks = build_chunks(&spans, total_s, job.chunk, &mut lowest);
    meta.total_chunks = chunks.len();
    meta.duration_s = total_s;
    if chunks.is_empty() {
        cp.write_meta(&meta)?;
        return Ok(Transcript {
            paragraphs: vec![],
            segments: vec![],
            text: String::new(),
            language: None,
            duration_s: total_s,
            speech_s: 0.0,
            word_count: 0,
            processing_s: t_start.elapsed().as_secs_f64(),
            chunks_total: 0,
            chunks_resumed: 0,
        });
    }

    // 3) language: majority vote over up to the first 3 chunks, then lock
    let mut lang_guess: Option<LangGuess> = None;
    let language: String = match (&job.language, &meta.locked_language) {
        (LanguageSetting::Code(c), _) => c.clone(),
        (LanguageSetting::Auto, Some(l)) => {
            lang_guess = Some(LangGuess {
                code: l.clone(),
                prob: meta.language_prob.unwrap_or(0.0),
            });
            l.clone()
        }
        (LanguageSetting::Auto, None) => {
            sink.event(SttEvent::Stage(Stage::DetectingLanguage));
            let mut guesses = Vec::new();
            for c in chunks.iter().take(3) {
                if cancel.load(Ordering::Relaxed) {
                    return Err(Error::Cancelled);
                }
                let n = ((c.len().min(30.0)) * STT_RATE as f64) as usize;
                pcm.read_f32((c.start_s * STT_RATE as f64) as u64, n, &mut scratch)?;
                if !should_transcribe(&analyze(&scratch)) {
                    continue;
                }
                normalize_peak(&mut scratch);
                guesses.push(engine.detect_language(&scratch, job.threads)?);
            }
            let g = vote_language(&guesses).unwrap_or(LangGuess {
                code: "en".into(),
                prob: 0.0,
            });
            meta.locked_language = Some(g.code.clone());
            meta.language_prob = Some(g.prob);
            lang_guess = Some(g.clone());
            g.code
        }
    };
    if let LanguageSetting::Code(c) = &job.language {
        lang_guess = Some(LangGuess {
            code: c.clone(),
            prob: 1.0,
        });
    }
    cp.write_meta(&meta)?;
    if let Some(g) = &lang_guess {
        sink.event(SttEvent::Language(g.clone()));
    }

    // 4) transcribe chunk by chunk
    sink.event(SttEvent::Stage(Stage::Transcribing));
    let total_audio: f64 = chunks.iter().map(|c| c.len()).sum();
    let mut done_audio = 0.0;
    let mut run_audio = 0.0; // audio transcribed in *this* run (for the rolling RTF)
    let t_run = Instant::now();
    let mut asm = Assembler::standard();
    let mut paragraphs = Vec::new();
    let mut all_segs: Vec<Seg> = Vec::new();
    let mut resumed = 0usize;
    let mut prev_tail = String::new();
    for (i, c) in chunks.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let segs: Vec<Seg> = if let Some(rec) = done_chunks.get(&i) {
            resumed += 1;
            rec.segments.clone()
        } else {
            pcm.read_f32(
                (c.start_s * STT_RATE as f64) as u64,
                (c.len() * STT_RATE as f64) as usize,
                &mut scratch,
            )?;
            let analysis = analyze(&scratch); // on the *raw* level: tells silence from quiet speech
            let (segs, skipped) = if !should_transcribe(&analysis) {
                (vec![], true)
            } else {
                normalize_peak(&mut scratch); // quiet recordings reach the model at a sensible level
                let opts = TranscribeOpts {
                    language: Some(language.clone()),
                    initial_prompt: if job.carry_prompt && !prev_tail.is_empty() {
                        Some(prev_tail.clone())
                    } else {
                        None
                    },
                    threads: job.threads,
                };
                let out = engine.transcribe(&scratch, &opts, cancel)?;
                let mut v = Vec::new();
                for s in out.segments {
                    if should_drop_segment(&s, &analysis) {
                        continue;
                    }
                    let text = collapse_repeats(s.text.trim());
                    if text.is_empty() {
                        continue;
                    }
                    v.push(Seg {
                        start_s: c.start_s + s.start_s,
                        end_s: c.start_s + s.end_s.max(s.start_s),
                        text,
                    });
                }
                (v, false)
            };
            cp.append(&ChunkRecord {
                index: i,
                start_s: c.start_s,
                end_s: c.end_s,
                skipped,
                segments: segs.clone(),
            })?;
            run_audio += c.len();
            segs
        };
        if job.carry_prompt {
            if let Some(last) = segs.last() {
                let t = &last.text;
                prev_tail = t
                    .chars()
                    .rev()
                    .take(224)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
            }
        }
        for s in segs {
            all_segs.push(s.clone());
            if let Some(p) = asm.push(&s) {
                sink.event(SttEvent::Paragraph(p.clone()));
                paragraphs.push(p);
            }
        }
        done_audio += c.len();
        let eta = if run_audio > 0.0 {
            let rtf = t_run.elapsed().as_secs_f64() / run_audio;
            Some(((total_audio - done_audio).max(0.0) * rtf * 1000.0) as u64)
        } else {
            None
        };
        sink.event(SttEvent::Progress {
            done_s: done_audio,
            total_s: total_audio,
            eta_ms: eta,
        });
    }
    if let Some(p) = asm.finish() {
        sink.event(SttEvent::Paragraph(p.clone()));
        paragraphs.push(p);
    }
    let text = paragraphs
        .iter()
        .map(|p| p.text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let word_count = text.split_whitespace().count();
    Ok(Transcript {
        paragraphs,
        segments: all_segs,
        text,
        language: lang_guess,
        duration_s: total_s,
        speech_s,
        word_count,
        processing_s: t_start.elapsed().as_secs_f64(),
        chunks_total: chunks.len(),
        chunks_resumed: resumed,
    })
}

fn fresh_meta(job: &SttJob, engine: &dyn SttEngine, lang_setting: &str) -> JobMeta {
    JobMeta {
        version: 1,
        input_key: job.input_key.clone(),
        model: engine.id().to_string(),
        language_setting: lang_setting.to_string(),
        locked_language: None,
        language_prob: None,
        total_chunks: 0,
        duration_s: 0.0,
    }
}

fn fnv1a(b: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for x in b {
        h ^= *x as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}
