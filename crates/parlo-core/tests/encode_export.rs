//! BRIEF §14.1 exports: WAV/MP3 decodable with duration within ±100 ms; >4 GB WAV guard.
mod common;
use parlo_core::audio::encode::*;
use parlo_core::audio::prepare::prepare_audio;
use parlo_core::audio::wav::{WavWriter, MAX_DATA_BYTES};
use parlo_core::error::Error;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

/// Write `secs` of a 24 kHz mono 220 Hz tone as a segment WAV.
fn tone_segment(path: &std::path::Path, secs: f64) {
    let n = (24_000.0 * secs) as usize;
    let mut w = WavWriter::create(path, 1, 24_000).unwrap();
    let pcm: Vec<i16> = (0..n)
        .map(|i| ((i as f32 / 24_000.0 * 220.0 * std::f32::consts::TAU).sin() * 12_000.0) as i16)
        .collect();
    w.write_i16(&pcm).unwrap();
    w.finalize().unwrap();
}

fn decoded_duration(path: &std::path::Path) -> f64 {
    let dir = tempfile::tempdir().unwrap();
    prepare_audio(
        path,
        &dir.path().join("d.wav"),
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap()
    .duration_s
}

#[test]
fn wav_export_concatenates_segments_exactly() {
    let dir = tempfile::tempdir().unwrap();
    let segs: Vec<PathBuf> = (0..3)
        .map(|i| dir.path().join(format!("s{i}.wav")))
        .collect();
    for (i, s) in segs.iter().enumerate() {
        tone_segment(s, 1.0 + i as f64);
    }
    let out = dir.path().join("out.wav");
    let frames = export_wav(&segs, &out).unwrap();
    assert_eq!(frames, 24_000 * 6);
    let r = hound::WavReader::open(&out).unwrap();
    assert_eq!(r.duration() as u64, frames);
    assert!((decoded_duration(&out) - 6.0).abs() < 0.1);
}

#[cfg(feature = "mp3")]
#[test]
fn mp3_export_is_decodable_with_matching_duration() {
    let dir = tempfile::tempdir().unwrap();
    let segs: Vec<PathBuf> = (0..2)
        .map(|i| dir.path().join(format!("s{i}.wav")))
        .collect();
    for s in &segs {
        tone_segment(s, 3.0);
    }
    let out = dir.path().join("out.mp3");
    export_mp3(&segs, &out, 96).unwrap();
    let size = std::fs::metadata(&out).unwrap().len();
    // 96 kbps CBR ≈ 12 kB/s
    assert!(
        (60_000..90_000).contains(&size),
        "{size} bytes for 6 s at 96 kbps"
    );
    let d = decoded_duration(&out);
    assert!((d - 6.0).abs() <= 0.1, "decoded duration {d}");
}

#[test]
fn wav_over_4gb_is_refused_with_a_helpful_message_before_writing() {
    // two sparse 2.5 GB "segments": valid headers, zero data — cheap on disk
    let dir = tempfile::tempdir().unwrap();
    let mut segs = Vec::new();
    for i in 0..2 {
        let p = dir.path().join(format!("big{i}.wav"));
        let w = WavWriter::create(&p, 1, 24_000).unwrap();
        w.finalize().unwrap();
        let f = std::fs::OpenOptions::new().write(true).open(&p).unwrap();
        f.set_len(44 + 2_500_000_000).unwrap();
        segs.push(p);
    }
    let out = dir.path().join("huge.wav");
    match export_wav(&segs, &out) {
        Err(Error::State(m)) => assert!(m.contains("M4A") && m.contains("4 GB"), "{m}"),
        other => panic!("expected a size-limit error, got {other:?}"),
    }
    assert!(!out.exists() && !out.with_extension("part").exists());
    const { assert!(5_000_000_000 > MAX_DATA_BYTES) };
}

#[test]
fn m4a_is_unavailable_on_linux_with_a_clear_error() {
    struct NoAac;
    impl parlo_core::platform::AacEncoder for NoAac {
        fn is_available(&self) -> bool {
            false
        }
        fn encode_wav_to_m4a(
            &self,
            _: &std::path::Path,
            _: &std::path::Path,
            _: u32,
        ) -> parlo_core::Result<()> {
            unreachable!()
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let r = export_m4a(&NoAac, &[], &dir.path().join("o.m4a"), 96, dir.path());
    assert!(matches!(r, Err(Error::Encode(_))));
}
