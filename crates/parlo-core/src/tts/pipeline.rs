//! Long-text pipeline (BRIEF §9.3): normalize → chunks → synthesize each chunk straight into the current segment WAV
//! (24 kHz mono PCM16) with generated silences; rotate segments (short at first so playback can start early);
//! checkpoint after every chunk; content-addressed output cache.

use super::engine::{Quality, TtsEngine};
use crate::audio::wav::WavWriter;
use crate::error::{Error, Result};
use crate::text::segment::{plan_chunks, Chunk, MAX_CHUNK_CHARS};
use crate::text::Block;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

/// Segment length targets: the first segment closes after ~25 s of audio so playback can start within seconds, then
/// 1 min, 2 min, 5 min, and finally ~10 min per segment (BRIEF §9.3 "about every 10 minutes").
pub fn segment_target_ms(index: usize) -> u64 {
    const T: [u64; 4] = [25_000, 60_000, 120_000, 300_000];
    T.get(index).copied().unwrap_or(600_000)
}

#[derive(Debug, Clone)]
pub struct TtsJob {
    pub blocks: Vec<Block>,
    pub voice: String,
    pub quality: Quality,
    /// where outputs live: `<outputs>/<output_id>/seg-0000.wav … + output.json`
    pub outputs_dir: PathBuf,
    /// `jobs/<id>/` checkpoint directory
    pub job_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TtsEvent {
    Progress {
        done_chars: usize,
        total_chars: usize,
        eta_ms: Option<u64>,
    },
    SegmentReady {
        index: usize,
        path: PathBuf,
        duration_ms: u64,
    },
    Done {
        output_id: String,
        duration_ms: u64,
    },
}

pub trait TtsSink {
    fn event(&mut self, e: TtsEvent);
}
impl<F: FnMut(TtsEvent)> TtsSink for F {
    fn event(&mut self, e: TtsEvent) {
        self(e)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SegmentInfo {
    pub file: String,
    pub duration_ms: u64,
    pub chunks: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutputMeta {
    pub version: u32,
    pub output_id: String,
    pub engine: String,
    pub model_revision: String,
    pub voice: String,
    pub quality: String,
    pub sample_rate: u32,
    pub total_chars: usize,
    pub segments: Vec<SegmentInfo>,
    pub duration_ms: u64,
    pub complete: bool,
}

#[derive(Debug, Clone)]
pub struct TtsOutput {
    pub output_id: String,
    pub dir: PathBuf,
    pub meta: OutputMeta,
    pub cache_hit: bool,
    pub segments: Vec<PathBuf>,
}

/// Output id = sha256(engine id + model revision + voice + quality + normalized text) (BRIEF §9.3.5).
pub fn output_id(
    engine: &dyn TtsEngine,
    voice: &str,
    quality: Quality,
    chunks: &[Chunk],
) -> String {
    let mut h = Sha256::new();
    for part in [
        engine.id(),
        engine.model_revision(),
        voice,
        quality.as_str(),
    ] {
        h.update(part.as_bytes());
        h.update([0u8]);
    }
    for c in chunks {
        h.update(c.text.as_bytes());
        h.update([if c.heading { 1 } else { 0 }]);
        h.update(c.pause_after_ms.to_le_bytes());
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Manifest {
    output_id: String,
    /// chunks fully synthesized (audio + trailing silence written, and synced)
    chunks_done: usize,
    /// closed segments
    segments: Vec<SegmentInfo>,
    /// the open segment (index = segments.len()) and how many frames of it are durable
    open_frames: u64,
    open_chunks: usize,
}

fn seg_name(i: usize) -> String {
    format!("seg-{i:04}.wav")
}

fn read_meta(dir: &Path) -> Option<OutputMeta> {
    serde_json::from_slice(&std::fs::read(dir.join("output.json")).ok()?).ok()
}

fn write_json(path: &Path, v: &impl Serialize) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(v)?).map_err(|e| Error::io(&tmp, e))?;
    std::fs::rename(&tmp, path).map_err(|e| Error::io(path, e))
}

/// Does a complete cached output exist (all segment files present with the recorded length)?
fn valid_cache(dir: &Path) -> Option<OutputMeta> {
    let m = read_meta(dir)?;
    if !m.complete {
        return None;
    }
    for s in &m.segments {
        let p = dir.join(&s.file);
        let info = repair_wav_readonly(&p)?;
        if info != s.duration_ms {
            return None;
        }
    }
    Some(m)
}

fn repair_wav_readonly(p: &Path) -> Option<u64> {
    let f = crate::audio::wav::PcmFile::open(p).ok()?;
    Some(f.frames * 1000 / f.sample_rate as u64)
}

pub fn run_tts(
    job: &TtsJob,
    engine: &mut dyn TtsEngine,
    sink: &mut dyn TtsSink,
    cancel: &AtomicBool,
) -> Result<TtsOutput> {
    let chunks = plan_chunks(&job.blocks, MAX_CHUNK_CHARS);
    if chunks.is_empty() {
        return Err(Error::Other("There is no text to read.".into()));
    }
    if !engine.voices().iter().any(|v| v.id == job.voice) {
        return Err(Error::Other(format!(
            "The voice '{}' is not available.",
            job.voice
        )));
    }
    let id = output_id(engine, &job.voice, job.quality, &chunks);
    let out_dir = job.outputs_dir.join(&id);
    let total_chars: usize = chunks.iter().map(|c| c.text.chars().count()).sum();
    let rate = engine.sample_rate();

    // cache hit: instant
    if let Some(meta) = valid_cache(&out_dir) {
        let segs: Vec<PathBuf> = meta
            .segments
            .iter()
            .map(|s| out_dir.join(&s.file))
            .collect();
        for (i, (s, p)) in meta.segments.iter().zip(&segs).enumerate() {
            sink.event(TtsEvent::SegmentReady {
                index: i,
                path: p.clone(),
                duration_ms: s.duration_ms,
            });
        }
        sink.event(TtsEvent::Progress {
            done_chars: total_chars,
            total_chars,
            eta_ms: Some(0),
        });
        sink.event(TtsEvent::Done {
            output_id: id.clone(),
            duration_ms: meta.duration_ms,
        });
        return Ok(TtsOutput {
            output_id: id,
            dir: out_dir,
            segments: segs,
            meta,
            cache_hit: true,
        });
    }

    std::fs::create_dir_all(&out_dir).map_err(|e| Error::io(&out_dir, e))?;
    std::fs::create_dir_all(&job.job_dir).map_err(|e| Error::io(&job.job_dir, e))?;
    let manifest_path = job.job_dir.join("manifest.json");
    let mut m: Manifest = std::fs::read(&manifest_path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .filter(|m: &Manifest| m.output_id == id)
        .unwrap_or_else(|| Manifest {
            output_id: id.clone(),
            ..Default::default()
        });

    // resume: closed segments must still exist; the open one is truncated back to its durable length
    let mut resumed = m.chunks_done > 0;
    if resumed {
        let ok_closed = m.segments.iter().all(|s| {
            repair_wav_readonly(&out_dir.join(&s.file)).is_some_and(|ms| ms == s.duration_ms)
        });
        let open_path = out_dir.join(seg_name(m.segments.len()));
        let ok_open =
            m.open_chunks == 0 || WavWriter::open_append(&open_path, m.open_frames).is_ok();
        if !(ok_closed && ok_open) {
            m = Manifest {
                output_id: id.clone(),
                ..Default::default()
            };
            resumed = false;
        }
    }
    if !resumed {
        for e in std::fs::read_dir(&out_dir)
            .map_err(|e| Error::io(&out_dir, e))?
            .flatten()
        {
            let _ = std::fs::remove_file(e.path());
        }
    }

    // replay already-finished segments to the sink so a resumed job rebuilds its playlist
    for (i, s) in m.segments.iter().enumerate() {
        sink.event(TtsEvent::SegmentReady {
            index: i,
            path: out_dir.join(&s.file),
            duration_ms: s.duration_ms,
        });
    }

    let mut chars_done: usize = chunks
        .iter()
        .take(m.chunks_done)
        .map(|c| c.text.chars().count())
        .sum();
    let mut writer: Option<WavWriter> = None;
    let open_idx = |m: &Manifest| m.segments.len();
    let silence_frames = |ms: u32| (rate as u64 * ms as u64 / 1000) as usize;
    let t_run = Instant::now();
    let mut chars_this_run = 0usize;
    let mut pcm: Vec<i16> = Vec::with_capacity(rate as usize);
    sink.event(TtsEvent::Progress {
        done_chars: chars_done,
        total_chars,
        eta_ms: None,
    });

    for (ci, chunk) in chunks.iter().enumerate().skip(m.chunks_done) {
        if cancel.load(Ordering::Relaxed) {
            if let Some(w) = writer.take() {
                let _ = w.finalize();
            }
            return Err(Error::Cancelled);
        }
        let w = match writer.as_mut() {
            Some(w) => w,
            None => {
                let path = out_dir.join(seg_name(open_idx(&m)));
                let w = if m.open_chunks > 0 {
                    WavWriter::open_append(&path, m.open_frames)?
                } else {
                    WavWriter::create(&path, 1, rate)?
                };
                writer = Some(w);
                writer.as_mut().unwrap()
            }
        };
        // synthesize straight to disk
        let res = engine.synthesize(
            &chunk.text,
            &job.voice,
            job.quality,
            &mut |samples: &[f32]| {
                pcm.clear();
                pcm.extend(
                    samples
                        .iter()
                        .map(|x| (x.clamp(-1.0, 1.0) * 32767.0).round() as i16),
                );
                w.write_i16(&pcm)
            },
            cancel,
        );
        if let Err(e) = res {
            // keep what was durably checkpointed; the half-written chunk is discarded by truncation on resume
            let _ = writer.take().map(|w| w.finalize());
            return Err(e);
        }
        // trailing silence (generated; PocketTTS cannot pause from text)
        let sil = vec![0i16; silence_frames(chunk.pause_after_ms)];
        w.write_i16(&sil)?;
        w.sync()?;
        m.chunks_done = ci + 1;
        m.open_chunks += 1;
        m.open_frames = w.frames();
        chars_done += chunk.text.chars().count();
        chars_this_run += chunk.text.chars().count();

        let seg_ms = w.frames() * 1000 / rate as u64;
        let last = ci + 1 == chunks.len();
        let heading_boundary = chunk.heading; // never leave a heading at the end of a segment: rotate before it instead
        let should_close = last || (seg_ms >= segment_target_ms(open_idx(&m)) && !heading_boundary);
        if should_close {
            let w = writer.take().unwrap();
            let frames = w.finalize()?;
            let info = SegmentInfo {
                file: seg_name(open_idx(&m)),
                duration_ms: frames * 1000 / rate as u64,
                chunks: m.open_chunks,
            };
            let idx = m.segments.len();
            m.segments.push(info.clone());
            m.open_frames = 0;
            m.open_chunks = 0;
            write_json(&manifest_path, &m)?;
            sink.event(TtsEvent::SegmentReady {
                index: idx,
                path: out_dir.join(&info.file),
                duration_ms: info.duration_ms,
            });
        } else {
            write_json(&manifest_path, &m)?;
        }
        let eta = if chars_this_run > 0 {
            let per_char = t_run.elapsed().as_secs_f64() / chars_this_run as f64;
            Some(((total_chars - chars_done) as f64 * per_char * 1000.0) as u64)
        } else {
            None
        };
        sink.event(TtsEvent::Progress {
            done_chars: chars_done,
            total_chars,
            eta_ms: eta,
        });
    }

    let duration_ms: u64 = m.segments.iter().map(|s| s.duration_ms).sum();
    let meta = OutputMeta {
        version: 1,
        output_id: id.clone(),
        engine: engine.id().into(),
        model_revision: engine.model_revision().into(),
        voice: job.voice.clone(),
        quality: job.quality.as_str().into(),
        sample_rate: rate,
        total_chars,
        segments: m.segments.clone(),
        duration_ms,
        complete: true,
    };
    write_json(&out_dir.join("output.json"), &meta)?;
    let _ = std::fs::remove_dir_all(&job.job_dir);
    sink.event(TtsEvent::Done {
        output_id: id.clone(),
        duration_ms,
    });
    let segments = meta
        .segments
        .iter()
        .map(|s| out_dir.join(&s.file))
        .collect();
    Ok(TtsOutput {
        output_id: id,
        dir: out_dir,
        meta,
        cache_hit: false,
        segments,
    })
}
