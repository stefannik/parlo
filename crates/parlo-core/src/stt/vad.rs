//! Voice-activity detection. `SpeechDetector` is the seam; the Silero implementation runs through whisper.cpp's VAD
//! (feature `whisper`). `detect_file` windows over a PCM file so memory stays bounded for any duration.

use crate::audio::wav::PcmFile;
use crate::error::{Error, Result};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub start_s: f64,
    pub end_s: f64,
}
impl Span {
    pub fn len(&self) -> f64 {
        self.end_s - self.start_s
    }
    pub fn is_empty(&self) -> bool {
        self.len() <= 0.0
    }
}

pub trait SpeechDetector: Send {
    /// Speech spans (seconds, relative to the clip start) in a mono 16 kHz clip.
    fn detect(&mut self, samples: &[f32]) -> Result<Vec<Span>>;
}

/// Window and overlap used when scanning a long file (BRIEF §7.2: "e.g. 5 min, with overlap").
pub const WINDOW_S: f64 = 300.0;
pub const OVERLAP_S: f64 = 5.0;

/// Detect speech over the whole file in overlapping windows; spans from adjacent windows that touch are merged.
pub fn detect_file(
    pcm: &PcmFile,
    det: &mut dyn SpeechDetector,
    cancel: &AtomicBool,
    min_silence_s: f64,
    mut progress: impl FnMut(f64),
) -> Result<Vec<Span>> {
    let rate = pcm.sample_rate as f64;
    let total_s = pcm.frames as f64 / rate;
    let mut out: Vec<Span> = Vec::new();
    let mut buf = Vec::new();
    let mut win_start = 0.0f64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let win_end = (win_start + WINDOW_S).min(total_s);
        pcm.read_f32(
            (win_start * rate) as u64,
            ((win_end - win_start) * rate) as usize,
            &mut buf,
        )?;
        let spans = det.detect(&buf)?;
        for s in spans {
            let (a, b) = (s.start_s + win_start, s.end_s + win_start);
            // drop what the previous window already covered completely (inside the overlap)
            if let Some(last) = out.last_mut() {
                if b <= last.end_s + 1e-6 {
                    continue;
                }
                if a <= last.end_s + min_silence_s {
                    last.end_s = last.end_s.max(b);
                    continue;
                }
            }
            out.push(Span {
                start_s: a,
                end_s: b,
            });
        }
        progress((win_end / total_s).clamp(0.0, 1.0));
        if win_end >= total_s - 1e-9 {
            break;
        }
        win_start = win_end - OVERLAP_S;
    }
    Ok(out)
}
