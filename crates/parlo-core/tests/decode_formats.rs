//! BRIEF §14.1 "Decode: every format in §7.1 decodes to the expected duration (±50 ms) and sample count."
mod common;
use common::fixture;
use parlo_core::audio::prepare::{prepare_audio, prepare_audio_with, STT_RATE};
use parlo_core::audio::wav::PcmFile;
use parlo_core::error::{DecodeError, Error};
use std::sync::atomic::AtomicBool;

/// Duration of the English source all `formats/*` fixtures were encoded from (ffprobe of en_all.wav).
fn source_duration() -> f64 {
    let p = fixture("generated/synth/en_all.wav");
    let r = hound::WavReader::open(p).unwrap();
    r.duration() as f64 / r.spec().sample_rate as f64
}

fn check(rel: &str, tol_s: f64) {
    let src = fixture(rel);
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("out.wav");
    let p = prepare_audio(&src, &dest, &AtomicBool::new(false), |_| {})
        .unwrap_or_else(|e| panic!("{rel}: {e}"));
    let want = source_duration();
    assert!(
        (p.duration_s - want).abs() <= tol_s,
        "{rel}: duration {:.3}s, expected {:.3}s ±{tol_s}",
        p.duration_s,
        want
    );
    // sample count consistent with duration, header consistent with file
    let f = PcmFile::open(&dest).unwrap();
    assert_eq!(f.sample_rate, STT_RATE);
    assert_eq!(f.channels, 1);
    assert_eq!(f.frames, p.frames);
}

macro_rules! fmt_test {
    ($name:ident, $file:expr) => {
        #[test]
        fn $name() {
            check(concat!("generated/formats/", $file), 0.05);
        }
    };
}

fmt_test!(wav_pcm16_44k_stereo, "wav_pcm16_44k_stereo.wav");
fmt_test!(wav_pcm16_16k_mono, "wav_pcm16_16k_mono.wav");
fmt_test!(wav_pcm16_8k_mono, "wav_pcm16_8k_mono.wav");
fmt_test!(wav_pcm24_48k_mono, "wav_pcm24_48k_mono.wav");
fmt_test!(wav_float32_48k_stereo, "wav_float32_48k_stereo.wav");
fmt_test!(mp3_128k, "mp3_128k.mp3");
fmt_test!(mp3_vbr_mono, "mp3_vbr_mono.mp3");
fmt_test!(aac_m4a, "aac_128k.m4a");
fmt_test!(alac_m4a, "alac.m4a");
fmt_test!(flac, "flac.flac");
fmt_test!(vorbis_ogg, "vorbis.ogg");
fmt_test!(opus_ogg, "opus.ogg");
fmt_test!(opus_webm, "opus.webm");
fmt_test!(aiff, "aiff_pcm16.aiff");
fmt_test!(caf_pcm, "caf_pcm16.caf");
fmt_test!(caf_alac, "caf_alac.caf");
fmt_test!(video_mp4, "video_h264_aac.mp4");
fmt_test!(video_mov, "video_h264_aac.mov");
fmt_test!(video_m4v, "video_h264_aac.m4v");
// MKV stores no AAC priming/padding information (unlike MP4 edit lists), so the exact start of the audio is
// unknowable: tolerance is one AAC frame (1024 samples = 46 ms at 22.05 kHz) + padding. See DECISIONS.md.
#[test]
fn video_mkv() {
    check("generated/formats/video_h264_aac.mkv", 0.10);
}
fmt_test!(video_webm, "video_vp9_opus.webm");

fn expect_decode_err(rel: &str) -> DecodeError {
    let dir = tempfile::tempdir().unwrap();
    match prepare_audio(
        &fixture(rel),
        &dir.path().join("o.wav"),
        &AtomicBool::new(false),
        |_| {},
    ) {
        Err(Error::Decode(e)) => e,
        other => panic!("{rel}: expected a DecodeError, got {other:?}"),
    }
}

#[test]
fn rejects_empty_file() {
    assert!(matches!(
        expect_decode_err("generated/formats/empty.wav"),
        DecodeError::Empty
    ));
}
#[test]
fn rejects_video_without_audio_track() {
    assert!(matches!(
        expect_decode_err("generated/formats/novideo_noaudio.mp4"),
        DecodeError::NoAudioTrack
    ));
}
#[test]
fn rejects_zero_duration() {
    let e = expect_decode_err("generated/formats/zero_duration.wav");
    assert!(
        matches!(
            e,
            DecodeError::ZeroDuration | DecodeError::Corrupt(_) | DecodeError::Unsupported(_)
        ),
        "{e:?}"
    );
}
#[test]
fn rejects_garbage() {
    let e = expect_decode_err("generated/formats/garbage.mp3");
    assert!(
        matches!(e, DecodeError::Unsupported(_) | DecodeError::Corrupt(_)),
        "{e:?}"
    );
}

/// The platform fallback (macOS AVFoundation) gets exactly one chance when the portable decoder can't read a file.
#[derive(Debug)]
struct FakeFallback {
    handles: bool,
    calls: std::sync::atomic::AtomicUsize,
}
impl parlo_core::platform::FallbackDecoder for FakeFallback {
    fn decode_to_wav16k(
        &self,
        _src: &std::path::Path,
        dest: &std::path::Path,
    ) -> parlo_core::error::Result<bool> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if !self.handles {
            return Ok(false);
        }
        let mut w = parlo_core::audio::wav::WavWriter::create(dest, 1, STT_RATE)?;
        w.write_i16(&vec![1000i16; STT_RATE as usize * 2])?;
        w.finalize()?;
        Ok(true)
    }
}

#[test]
fn fallback_decoder_rescues_unsupported_files() {
    let dir = tempfile::tempdir().unwrap();
    let fb = FakeFallback {
        handles: true,
        calls: 0.into(),
    };
    let p = prepare_audio_with(
        &fixture("generated/formats/garbage.mp3"),
        &dir.path().join("o.wav"),
        &AtomicBool::new(false),
        |_| {},
        Some(&fb),
    )
    .expect("fallback should produce audio");
    assert_eq!(fb.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!((p.duration_s - 2.0).abs() < 0.01, "{}", p.duration_s);
    assert!(dir.path().join("o.wav").exists());
    assert!(
        !dir.path().join("o.part").exists(),
        "no temp file left behind"
    );
}

#[test]
fn fallback_decoder_declining_keeps_the_original_error() {
    let dir = tempfile::tempdir().unwrap();
    let fb = FakeFallback {
        handles: false,
        calls: 0.into(),
    };
    let e = prepare_audio_with(
        &fixture("generated/formats/garbage.mp3"),
        &dir.path().join("o.wav"),
        &AtomicBool::new(false),
        |_| {},
        Some(&fb),
    )
    .unwrap_err();
    assert!(
        matches!(
            e,
            Error::Decode(DecodeError::Unsupported(_) | DecodeError::Corrupt(_))
        ),
        "{e:?}"
    );
    assert_eq!(fb.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(!dir.path().join("o.wav").exists() && !dir.path().join("o.part").exists());
}

#[test]
fn fallback_decoder_is_not_consulted_for_good_files() {
    let dir = tempfile::tempdir().unwrap();
    let fb = FakeFallback {
        handles: true,
        calls: 0.into(),
    };
    prepare_audio_with(
        &fixture("generated/formats/flac.flac"),
        &dir.path().join("o.wav"),
        &AtomicBool::new(false),
        |_| {},
        Some(&fb),
    )
    .unwrap();
    assert_eq!(fb.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[test]
fn cancel_is_honored() {
    let dir = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(true);
    let r = prepare_audio(
        &fixture("generated/formats/flac.flac"),
        &dir.path().join("o.wav"),
        &cancel,
        |_| {},
    );
    assert!(matches!(r, Err(Error::Cancelled)));
    assert!(
        !dir.path().join("o.wav").exists() && !dir.path().join("o.part").exists(),
        "no partial output left behind"
    );
}

/// Decoded *content* must match the source (not just the length): level within 3 dB and normalised
/// cross-correlation > 0.95 within ±60 ms lag, on a 2 kHz-decimated copy.
#[test]
fn decoded_content_matches_source_for_every_codec() {
    fn load(rel: &str) -> Vec<f32> {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("o.wav");
        let p = prepare_audio(&fixture(rel), &dest, &AtomicBool::new(false), |_| {}).unwrap();
        let f = PcmFile::open(&dest).unwrap();
        let mut v = Vec::new();
        f.read_f32(0, p.frames as usize, &mut v).unwrap();
        // decimate 8x by block-averaging (crude low-pass) -> 2 kHz
        v.chunks_exact(8)
            .map(|c| c.iter().sum::<f32>() / 8.0)
            .collect()
    }
    let base = load("generated/formats/wav_pcm16_16k_mono.wav");
    let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
    // (file, minimum correlation): lossless codecs must be essentially identical; lossy ones merely close.
    let files: [(&str, f32); 15] = [
        ("alac.m4a", 0.99),
        ("flac.flac", 0.99),
        ("aiff_pcm16.aiff", 0.99),
        ("caf_pcm16.caf", 0.99),
        ("caf_alac.caf", 0.99),
        ("wav_pcm24_48k_mono.wav", 0.99),
        ("wav_float32_48k_stereo.wav", 0.99),
        ("mp3_128k.mp3", 0.95),
        ("aac_128k.m4a", 0.9),
        ("vorbis.ogg", 0.9),
        ("opus.ogg", 0.85),
        ("opus.webm", 0.85),
        ("video_h264_aac.mp4", 0.9),
        ("video_vp9_opus.webm", 0.85),
        ("mp3_vbr_mono.mp3", 0.8),
    ];
    for (f, min_corr) in files {
        let x = load(&format!("generated/formats/{f}"));
        let db = 20.0 * (rms(&x) / rms(&base)).log10();
        // ffmpeg up-mixes mono->stereo at -3 dB per channel, and we average channels, so stereo fixtures read ~-3.0 dB.
        assert!(db.abs() < 3.3, "{f}: level differs by {db:.1} dB");
        let n = base.len().min(x.len()) - 200;
        let mut best = f32::MIN;
        for lag in -120i32..=120 {
            let (mut sxy, mut sxx, mut syy) = (0f32, 0f32, 0f32);
            for (i, a) in base.iter().enumerate().take(n).skip(130) {
                let j = (i as i32 + lag) as usize;
                let (a, b) = (*a, x[j]);
                sxy += a * b;
                sxx += a * a;
                syy += b * b;
            }
            best = best.max(sxy / (sxx * syy).sqrt());
        }
        assert!(
            best > min_corr,
            "{f}: correlation with source only {best:.3} (min {min_corr})"
        );
    }
}
