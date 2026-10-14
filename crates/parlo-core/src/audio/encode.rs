//! Export encoders (BRIEF §9.4). All stream block-wise across segment WAVs, so memory stays bounded.
//! WAV: hound-free streaming concat with the >4 GB guard. MP3: LAME (feature `mp3`). M4A: platform `AacEncoder`.

use super::wav::{PcmFile, WavWriter, MAX_DATA_BYTES};
use crate::error::{Error, Result};
use crate::platform::AacEncoder;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Wav,
    Mp3,
    M4a,
}

const BLOCK: usize = 16 * 1024;

/// Total PCM data bytes of the segments, and their common format (rate, channels). Errors if formats differ.
pub fn inspect_segments(segments: &[PathBuf]) -> Result<(u64, u32, u16)> {
    let mut total = 0u64;
    let mut fmt: Option<(u32, u16)> = None;
    for s in segments {
        let f = PcmFile::open(s)?;
        match fmt {
            None => fmt = Some((f.sample_rate, f.channels)),
            Some(x) if x != (f.sample_rate, f.channels) => {
                return Err(Error::State("segments have different audio formats".into()))
            }
            _ => {}
        }
        total += f.frames * 2 * f.channels as u64;
    }
    let (r, c) = fmt.ok_or_else(|| Error::State("nothing to export".into()))?;
    Ok((total, r, c))
}

fn for_each_block(segments: &[PathBuf], mut f: impl FnMut(&[i16]) -> Result<()>) -> Result<()> {
    let mut buf: Vec<i16> = Vec::with_capacity(BLOCK);
    for s in segments {
        let mut r = hound::WavReader::open(s)?;
        for sample in r.samples::<i16>() {
            buf.push(sample?);
            if buf.len() == BLOCK {
                f(&buf)?;
                buf.clear();
            }
        }
    }
    if !buf.is_empty() {
        f(&buf)?;
    }
    Ok(())
}

/// Concatenate PCM16 segment WAVs into one WAV. Refuses (before writing) if the result exceeds the 4 GB RIFF limit.
pub fn export_wav(segments: &[PathBuf], dest: &Path) -> Result<u64> {
    let (bytes, rate, ch) = inspect_segments(segments)?;
    if bytes > MAX_DATA_BYTES {
        return Err(Error::State(format!(
            "This audio is {:.1} GB, which is too large for a WAV file (limit 4 GB). Export as M4A instead.",
            bytes as f64 / 1e9
        )));
    }
    let tmp = dest.with_extension("part");
    let mut w = WavWriter::create(&tmp, ch, rate)?;
    let res = for_each_block(segments, |b| w.write_i16(b));
    match res {
        Ok(()) => {
            let frames = w.finalize()?;
            std::fs::rename(&tmp, dest).map_err(|e| Error::io(dest, e))?;
            Ok(frames)
        }
        Err(e) => {
            drop(w);
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Encode mono segments to CBR MP3 (default 96 kbps).
#[cfg(feature = "mp3")]
pub fn export_mp3(segments: &[PathBuf], dest: &Path, kbps: u32) -> Result<()> {
    use mp3lame_encoder::{Bitrate, Builder, FlushNoGap, MonoPcm, Quality};
    use std::io::Write;
    let (_, rate, ch) = inspect_segments(segments)?;
    if ch != 1 {
        return Err(Error::Encode("MP3 export expects mono audio".into()));
    }
    let br = match kbps {
        64 => Bitrate::Kbps64,
        96 => Bitrate::Kbps96,
        128 => Bitrate::Kbps128,
        160 => Bitrate::Kbps160,
        192 => Bitrate::Kbps192,
        _ => return Err(Error::Encode(format!("unsupported MP3 bitrate {kbps}"))),
    };
    let mut b =
        Builder::new().ok_or_else(|| Error::Encode("could not create the MP3 encoder".into()))?;
    let e = |x: &dyn std::fmt::Debug| Error::Encode(format!("{x:?}"));
    b.set_num_channels(1).map_err(|x| e(&x))?;
    b.set_sample_rate(rate).map_err(|x| e(&x))?;
    b.set_brate(br).map_err(|x| e(&x))?;
    b.set_quality(Quality::Good).map_err(|x| e(&x))?;
    let mut enc = b.build().map_err(|x| e(&x))?;
    let tmp = dest.with_extension("part");
    let mut out =
        std::io::BufWriter::new(std::fs::File::create(&tmp).map_err(|x| Error::io(&tmp, x))?);
    let mut buf: Vec<u8> = Vec::new();
    let res = (|| -> Result<()> {
        for_each_block(segments, |pcm| {
            buf.clear();
            buf.reserve(mp3lame_encoder::max_required_buffer_size(pcm.len()));
            enc.encode_to_vec(MonoPcm(pcm), &mut buf)
                .map_err(|x| e(&x))?;
            out.write_all(&buf).map_err(|x| Error::io(&tmp, x))
        })?;
        buf.clear();
        buf.reserve(7200);
        enc.flush_to_vec::<FlushNoGap>(&mut buf)
            .map_err(|x| e(&x))?;
        out.write_all(&buf).map_err(|x| Error::io(&tmp, x))?;
        out.flush().map_err(|x| Error::io(&tmp, x))
    })();
    drop(out);
    match res {
        Ok(()) => std::fs::rename(&tmp, dest).map_err(|x| Error::io(dest, x)),
        Err(er) => {
            let _ = std::fs::remove_file(&tmp);
            Err(er)
        }
    }
}

/// M4A/AAC via the platform encoder (AudioToolbox on macOS). Segments are first concatenated into a temp WAV.
pub fn export_m4a(
    aac: &dyn AacEncoder,
    segments: &[PathBuf],
    dest: &Path,
    kbps: u32,
    tmp_dir: &Path,
) -> Result<()> {
    if !aac.is_available() {
        return Err(Error::Encode(
            "M4A export is not available on this platform".into(),
        ));
    }
    std::fs::create_dir_all(tmp_dir).map_err(|e| Error::io(tmp_dir, e))?;
    let tmp_wav = tmp_dir.join(format!("export-{}.wav", std::process::id()));
    let r = (|| -> Result<()> {
        // an M4A export of > 4 GB of PCM can't go through a classic WAV; AudioToolbox could read segments directly
        // (macOS implementation may override); for ≤ 4 GB this path is exact.
        export_wav(segments, &tmp_wav)?;
        aac.encode_wav_to_m4a(&tmp_wav, dest, kbps)
    })();
    let _ = std::fs::remove_file(&tmp_wav);
    r
}
