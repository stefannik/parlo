//! Prepare any supported input for STT: decode → downmix → resample to 16 kHz → mono PCM16 WAV on disk.
//! Memory is bounded (block-wise streaming); the output is a plain WAV so tools can inspect it.

use super::decode::{decode_mono, AudioInfo};
use super::resample::StreamResampler;
use super::wav::WavWriter;
use crate::error::{Error, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

pub const STT_RATE: u32 = 16_000;

#[derive(Debug, Clone)]
pub struct Prepared {
    pub path: PathBuf,
    pub frames: u64,
    pub duration_s: f64,
    pub source: AudioInfo,
}

pub fn f32_to_i16(x: f32) -> i16 {
    (x.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

/// Decode `src` into `dest` (16 kHz mono PCM16 WAV). `progress` gets 0..=1 (if the container knows its length).
pub fn prepare_audio(src: &Path, dest: &Path, cancel: &AtomicBool, progress: impl FnMut(f64)) -> Result<Prepared> {
    let tmp = dest.with_extension("part");
    let mut writer = WavWriter::create(&tmp, 1, STT_RATE)?;
    let mut rs: Option<StreamResampler> = None;
    let mut cur_rate = 0u32;
    let mut out: Vec<f32> = Vec::new();
    let mut pcm: Vec<i16> = Vec::new();
    let res = decode_mono(src, cancel, progress, |block, rate| {
        if rs.is_none() || rate != cur_rate {
            if let Some(old) = rs.as_mut() {
                out.clear();
                old.finish(&mut out)?;
                pcm.clear();
                pcm.extend(out.iter().map(|x| f32_to_i16(*x)));
                writer.write_i16(&pcm)?;
            }
            rs = Some(StreamResampler::new(rate, STT_RATE)?);
            cur_rate = rate;
        }
        out.clear();
        rs.as_mut().unwrap().process(block, &mut out)?;
        pcm.clear();
        pcm.extend(out.iter().map(|x| f32_to_i16(*x)));
        writer.write_i16(&pcm)
    });
    let (info, _decoded) = match res {
        Ok(v) => v,
        Err(e) => {
            drop(writer);
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    };
    if let Some(r) = rs.as_mut() {
        out.clear();
        r.finish(&mut out)?;
        pcm.clear();
        pcm.extend(out.iter().map(|x| f32_to_i16(*x)));
        writer.write_i16(&pcm)?;
    }
    let frames = writer.finalize()?;
    if frames == 0 {
        let _ = std::fs::remove_file(&tmp);
        return Err(crate::error::DecodeError::ZeroDuration.into());
    }
    std::fs::rename(&tmp, dest).map_err(|e| Error::io(dest, e))?;
    Ok(Prepared { path: dest.to_path_buf(), frames, duration_s: frames as f64 / STT_RATE as f64, source: info })
}
