//! Prepare any supported input for STT: decode → downmix → resample to 16 kHz → mono PCM16 WAV on disk.
//! Memory is bounded (block-wise streaming); the output is a plain WAV so tools can inspect it.

use super::decode::{decode_mono, AudioInfo};
use super::resample::StreamResampler;
use super::wav::{PcmFile, WavWriter};
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

/// [`prepare_audio`] with a platform fallback (macOS: AVFoundation): when the portable decoder reports the container or
/// codec as unsupported/damaged, the fallback gets one chance before the error is surfaced.
pub fn prepare_audio_with(
    src: &Path,
    dest: &Path,
    cancel: &AtomicBool,
    progress: impl FnMut(f64),
    fallback: Option<&dyn crate::platform::FallbackDecoder>,
) -> Result<Prepared> {
    match prepare_audio(src, dest, cancel, progress) {
        Err(Error::Decode(
            crate::error::DecodeError::Unsupported(_) | crate::error::DecodeError::Corrupt(_),
        )) if fallback.is_some() => {
            let tmp = dest.with_extension("part");
            let handled = fallback
                .expect("checked above")
                .decode_to_wav16k(src, &tmp)
                .unwrap_or(false);
            let frames = if handled {
                PcmFile::open(&tmp).map(|f| f.frames).unwrap_or(0)
            } else {
                0
            };
            if frames == 0 {
                let _ = std::fs::remove_file(&tmp);
                // report the portable decoder's verdict, not the fallback's silence
                return prepare_audio(src, dest, cancel, |_| {});
            }
            std::fs::rename(&tmp, dest).map_err(|e| Error::io(dest, e))?;
            Ok(Prepared {
                path: dest.to_path_buf(),
                frames,
                duration_s: frames as f64 / STT_RATE as f64,
                source: AudioInfo {
                    sample_rate: STT_RATE,
                    channels: 1,
                    duration_s: Some(frames as f64 / STT_RATE as f64),
                    codec: "platform fallback".into(),
                },
            })
        }
        other => other,
    }
}

/// Decode `src` into `dest` (16 kHz mono PCM16 WAV). `progress` gets 0..=1 (if the container knows its length).
pub fn prepare_audio(
    src: &Path,
    dest: &Path,
    cancel: &AtomicBool,
    progress: impl FnMut(f64),
) -> Result<Prepared> {
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
    Ok(Prepared {
        path: dest.to_path_buf(),
        frames,
        duration_s: frames as f64 / STT_RATE as f64,
        source: info,
    })
}
