//! Streaming audio decode via symphonia (+ pure-Rust Opus adapter). Output: mono f32 blocks, bounded memory.
//! macOS builds can plug an AVFoundation fallback (see `platform::FallbackDecoder`).

use crate::error::{DecodeError, Error, Result};
use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use symphonia::core::audio::GenericAudioBufferRef;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::codecs::registry::CodecRegistry;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::default::get_probe;

#[derive(Debug, Clone, PartialEq)]
pub struct AudioInfo {
    pub sample_rate: u32,
    pub channels: u16,
    /// Container-reported duration in seconds, when known.
    pub duration_s: Option<f64>,
    pub codec: String,
}

fn registry() -> &'static CodecRegistry {
    use std::sync::OnceLock;
    static REG: OnceLock<CodecRegistry> = OnceLock::new();
    REG.get_or_init(|| {
        let mut r = CodecRegistry::new();
        symphonia::default::register_enabled_codecs(&mut r);
        r.register_audio_decoder::<symphonia_adapter_opus_rs::OpusAdapter>();
        r
    })
}

fn map_err(e: SymError, what: &str) -> Error {
    match e {
        SymError::Unsupported(m) => DecodeError::Unsupported(format!("{what}: {m}")).into(),
        SymError::IoError(ioe) if ioe.kind() == std::io::ErrorKind::UnexpectedEof => {
            DecodeError::Corrupt("the file ends unexpectedly".into()).into()
        }
        SymError::IoError(ioe) => Error::IoBare(ioe),
        other => DecodeError::Corrupt(format!("{what}: {other}")).into(),
    }
}

/// Decode `path` to mono f32, calling `sink(block, sample_rate)` for every decoded block.
/// `progress(fraction)` is called with 0..=1 when the container reports a duration (else not called).
/// Honors `cancel` between packets. Returns stream info and the number of mono frames decoded.
pub fn decode_mono(
    path: &Path,
    cancel: &AtomicBool,
    mut progress: impl FnMut(f64),
    mut sink: impl FnMut(&[f32], u32) -> Result<()>,
) -> Result<(AudioInfo, u64)> {
    let file = File::open(path).map_err(|e| Error::io(path, e))?;
    let len = file.metadata().map_err(|e| Error::io(path, e))?.len();
    if len == 0 {
        return Err(DecodeError::Empty.into());
    }
    let mss = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut format = get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| match e {
            SymError::Unsupported(_) => Error::from(DecodeError::Unsupported(
                "unrecognised or damaged file".into(),
            )),
            other => map_err(other, "opening the file"),
        })?;

    let track = format
        .default_track(TrackType::Audio)
        .ok_or(DecodeError::NoAudioTrack)?
        .clone();
    let params = match &track.codec_params {
        Some(symphonia::core::codecs::CodecParameters::Audio(p)) => p.clone(),
        _ => return Err(DecodeError::NoAudioTrack.into()),
    };
    let mut decoder = registry()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|e| map_err(e, "audio codec"))?;
    let tb = track.time_base;
    let duration_s = match (tb, track.num_frames.or(track.duration.map(|d| d.get()))) {
        (Some(tb), Some(n)) => tb.calc_duration(n.into()).map(|t| t.as_secs_f64()),
        _ => None,
    }
    .filter(|d| *d > 0.0);

    let mut info = AudioInfo {
        sample_rate: params.sample_rate.unwrap_or(0),
        channels: params
            .channels
            .as_ref()
            .map(|c| c.count() as u16)
            .unwrap_or(0),
        duration_s,
        codec: format!("{:?}", params.codec),
    };
    let track_id = track.id;
    // ISO-MP4 edit list (AAC priming) — symphonia's isomp4 reader is not gapless.
    let is_mp4 = format.format_info().short_name.contains("mp4")
        || format.format_info().short_name.contains("isomp4");
    let edit = if is_mp4 {
        super::mp4_edit::audio_edit(path, track_id)
    } else {
        None
    };
    let mut skip_frames: Option<u64> = None; // computed once the decoded rate is known
    let mut keep_frames: Option<u64> = None;
    let mut interleaved: Vec<f32> = Vec::new();
    let mut mono: Vec<f32> = Vec::new();
    let mut total: u64 = 0;
    let mut last_progress = -1.0f64;
    let mut consecutive_decode_errors = 0u32;

    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymError::ResetRequired) => break,
            Err(e) => return Err(map_err(e, "reading the file")),
        };
        if packet.track_id != track_id {
            continue;
        }
        let trim_s = tb
            .map(|tb| tb.calc_duration_saturating(packet.trim_start).as_secs_f64())
            .unwrap_or(0.0);
        let trim_e = tb
            .map(|tb| tb.calc_duration_saturating(packet.trim_end).as_secs_f64())
            .unwrap_or(0.0);
        let buf: GenericAudioBufferRef<'_> = match decoder.decode(&packet) {
            Ok(b) => b,
            Err(SymError::DecodeError(_)) | Err(SymError::IoError(_)) => {
                // Undecodable packet: skip (documented symphonia behaviour). Give up if the stream is mostly garbage.
                consecutive_decode_errors += 1;
                if consecutive_decode_errors > 200 {
                    return Err(DecodeError::Corrupt("too many undecodable packets".into()).into());
                }
                continue;
            }
            Err(e) => return Err(map_err(e, "decoding")),
        };
        consecutive_decode_errors = 0;
        let spec = buf.spec().clone();
        let ch = spec.channels().count().max(1);
        let rate = spec.rate();
        if info.sample_rate == 0 {
            info.sample_rate = rate;
        }
        if info.channels == 0 {
            info.channels = ch as u16;
        }
        interleaved.clear();
        buf.copy_to_vec_interleaved::<f32>(&mut interleaved);
        let frames = interleaved.len() / ch;
        mono.clear();
        mono.reserve(frames);
        if ch == 1 {
            mono.extend_from_slice(&interleaved);
        } else {
            let inv = 1.0 / ch as f32;
            for f in interleaved.chunks_exact(ch) {
                mono.push(f.iter().sum::<f32>() * inv);
            }
        }
        let mut start = ((trim_s * rate as f64).round() as usize).min(mono.len());
        let mut end = mono
            .len()
            .saturating_sub((trim_e * rate as f64).round() as usize)
            .max(start);
        if let Some(e) = edit {
            if skip_frames.is_none() {
                skip_frames = Some((e.skip_s * rate as f64).round() as u64);
                keep_frames = e.keep_s.map(|k| (k * rate as f64).round() as u64);
            }
            // drop priming frames still owed
            let owed = skip_frames.unwrap_or(0) as usize;
            let drop_now = owed.min(end - start);
            start += drop_now;
            skip_frames = Some((owed - drop_now) as u64);
            if let Some(k) = keep_frames {
                let take = (k as usize).min(end - start);
                end = start + take;
                keep_frames = Some(k - take as u64);
            }
        }
        let block = &mono[start..end];
        if !block.is_empty() {
            sink(block, rate)?;
            total += block.len() as u64;
        }
        if edit.is_some() && keep_frames == Some(0) {
            break; // presentation duration reached; the rest is encoder padding
        }
        if let (Some(d), true) = (info.duration_s, rate > 0) {
            let p = (total as f64 / rate as f64 / d).clamp(0.0, 1.0);
            if p - last_progress >= 0.005 {
                progress(p);
                last_progress = p;
            }
        }
    }
    if total == 0 {
        return Err(DecodeError::ZeroDuration.into());
    }
    progress(1.0);
    Ok((info, total))
}

/// Cheap probe for UI (duration, rate, channels). Does not decode.
pub fn probe_info(path: &Path) -> Result<AudioInfo> {
    let file = File::open(path).map_err(|e| Error::io(path, e))?;
    if file.metadata().map_err(|e| Error::io(path, e))?.len() == 0 {
        return Err(DecodeError::Empty.into());
    }
    let mss = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let format = get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|_| {
            Error::from(DecodeError::Unsupported(
                "unrecognised or damaged file".into(),
            ))
        })?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or(DecodeError::NoAudioTrack)?;
    let params = match &track.codec_params {
        Some(symphonia::core::codecs::CodecParameters::Audio(p)) => p,
        _ => return Err(DecodeError::NoAudioTrack.into()),
    };
    let duration_s = match (
        track.time_base,
        track.num_frames.or(track.duration.map(|d| d.get())),
    ) {
        (Some(tb), Some(n)) => tb.calc_duration(n.into()).map(|t| t.as_secs_f64()),
        _ => None,
    };
    Ok(AudioInfo {
        sample_rate: params.sample_rate.unwrap_or(0),
        channels: params
            .channels
            .as_ref()
            .map(|c| c.count() as u16)
            .unwrap_or(0),
        duration_s,
        codec: format!("{:?}", params.codec),
    })
}

/// The title stored in the file's own tags (ID3 `TIT2`, MP4 `©nam`, Vorbis `TITLE`, …), if any. Reads only the
/// container headers.
pub fn tag_title(path: &Path) -> Option<String> {
    use symphonia::core::meta::StandardTag;
    let file = File::open(path).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut format = get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .ok()?;
    let mut md = format.metadata();
    let rev = md.skip_to_latest()?;
    rev.media
        .tags
        .iter()
        .chain(rev.per_track.iter().flat_map(|t| t.metadata.tags.iter()))
        .find_map(|t| match &t.std {
            Some(StandardTag::TrackTitle(s)) => Some(s.trim().to_string()),
            _ => None,
        })
        .filter(|s| !s.is_empty())
}
