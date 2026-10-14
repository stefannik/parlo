//! macOS-only platform tests (MAC_HANDOFF sections B/F). The whole file is empty on other OSes.
#![cfg(target_os = "macos")]

use parlo_core::audio::prepare::{prepare_audio, prepare_audio_with};
use parlo_core::audio::wav::PcmFile;
use parlo_core::platform::{AacEncoder, FallbackDecoder, PdfTextExtractor, PowerGuard};
use parlo_core::text::pdf::{clean_pdf_pages, NO_TEXT_LAYER};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

fn fixture(rel: &str) -> PathBuf {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(rel);
    assert!(
        p.exists(),
        "fixture {} missing — scripts/gen-fixtures.sh",
        p.display()
    );
    p
}

fn dur_of(p: &Path, dir: &Path) -> f64 {
    let wav = dir.join(format!(
        "{}.dur.wav",
        p.file_name().unwrap().to_string_lossy()
    ));
    prepare_audio(p, &wav, &AtomicBool::new(false), |_| {}).unwrap();
    let f = PcmFile::open(&wav).unwrap();
    f.frames as f64 / f.sample_rate as f64
}

#[test]
fn pdfkit_import_matches_the_committed_snapshot() {
    let i = parlo_core::text::import::import_file(
        &fixture("text/headers_hyphens.pdf"),
        Some(&parlo_macos::pdf::PdfKitExtractor),
    )
    .unwrap();
    let expected =
        std::fs::read_to_string(fixture("text/expected/pdf_headers_hyphens_pdfkit.txt")).unwrap();
    assert_eq!(i.text(), expected);
    assert_eq!(i.stats.pages, Some(4));
}

#[test]
fn pdfkit_scanned_pdf_has_no_text_layer() {
    let pages = parlo_macos::pdf::PdfKitExtractor
        .page_texts(&fixture("text/scanned.pdf"))
        .unwrap();
    match clean_pdf_pages(&pages) {
        Err(parlo_core::Error::Other(m)) => assert_eq!(m, NO_TEXT_LAYER),
        other => panic!("{:?}", other.map(|c| c.blocks.len())),
    }
}

#[test]
fn pdfkit_reports_a_damaged_pdf_clearly() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("bad.pdf");
    std::fs::write(&p, b"not a pdf").unwrap();
    assert!(parlo_macos::pdf::PdfKitExtractor.page_texts(&p).is_err());
}

#[test]
fn audiotoolbox_m4a_is_decodable_with_matching_duration() {
    let d = tempfile::tempdir().unwrap();
    let enc = parlo_macos::aac::AudioToolboxAac;
    assert!(enc.is_available());
    let src = fixture("generated/synth/en_all.wav");
    // the encoder takes mono PCM16: normalise through the portable decoder first
    let wav = d.path().join("in.wav");
    prepare_audio(&src, &wav, &AtomicBool::new(false), |_| {}).unwrap();
    let want = dur_of(&wav, d.path());
    let m4a = d.path().join("out.m4a");
    enc.encode_wav_to_m4a(&wav, &m4a, 96).unwrap();
    assert!(std::fs::metadata(&m4a).unwrap().len() > 2000);
    let got = dur_of(&m4a, d.path());
    assert!((got - want).abs() <= 0.1, "m4a {got:.3}s vs wav {want:.3}s");
}

#[test]
fn avfoundation_fallback_decodes_a_video_and_an_m4a() {
    let d = tempfile::tempdir().unwrap();
    for f in [
        "video_h264_aac.mov",
        "aac_128k.m4a",
        "caf_alac.caf",
        "mp3_128k.mp3",
    ] {
        let src = fixture(&format!("generated/formats/{f}"));
        let want = dur_of(&src, d.path());
        let out = d.path().join(format!("{f}.av.wav"));
        let handled = parlo_macos::decode::AvFoundationDecoder
            .decode_to_wav16k(&src, &out)
            .unwrap();
        assert!(handled, "{f} not handled");
        let pcm = PcmFile::open(&out).unwrap();
        assert_eq!(pcm.sample_rate, 16_000);
        let got = pcm.frames as f64 / 16_000.0;
        assert!(
            (got - want).abs() <= 0.1,
            "{f}: AVFoundation {got:.3}s vs symphonia {want:.3}s"
        );
    }
}

#[test]
fn fallback_is_used_when_the_portable_decoder_gives_up() {
    // A file with an audio extension that symphonia rejects but AVFoundation also rejects must still fail cleanly
    let d = tempfile::tempdir().unwrap();
    let bad = d.path().join("garbage.mp3");
    std::fs::copy(fixture("generated/formats/garbage.mp3"), &bad).unwrap();
    let r = prepare_audio_with(
        &bad,
        &d.path().join("o.wav"),
        &AtomicBool::new(false),
        |_| {},
        Some(&parlo_macos::decode::AvFoundationDecoder),
    );
    assert!(r.is_err());
}

fn assertions() -> String {
    String::from_utf8_lossy(
        &std::process::Command::new("pmset")
            .args(["-g", "assertions"])
            .output()
            .unwrap()
            .stdout,
    )
    .into()
}

#[test]
fn power_guard_holds_a_sleep_assertion_while_alive() {
    let reason = format!("parlo-test-{}", std::process::id());
    let g = parlo_macos::power::MacPowerGuard.begin(&reason);
    let during = assertions();
    assert!(during.contains(&reason), "assertion missing:\n{during}");
    drop(g);
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(!assertions().contains(&reason), "assertion not released");
}

#[test]
fn mic_permission_state_is_readable_without_prompting() {
    let s = parlo_macos::mic::authorization_status();
    println!("mic status of the test binary: {s:?}");
}

#[test]
fn sleep_wake_and_memory_pressure_sources_can_be_created_and_subscribed() {
    use parlo_core::platform::{MemoryPressure, SleepWakeEvents};
    let sw = parlo_macos::sleepwake::MacSleepWake::new();
    let _rx = sw.subscribe();
    let mp = parlo_macos::memory::MacMemoryPressure::new();
    let _rx2 = mp.subscribe();
}

#[test]
#[ignore = "dev helper: writes /tmp/aac/out.m4a"]
fn write_sample_m4a() {
    let d = tempfile::tempdir().unwrap();
    let wav = d.path().join("in.wav");
    prepare_audio(
        &fixture("generated/synth/en_all.wav"),
        &wav,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    std::fs::create_dir_all("/tmp/aac").unwrap();
    parlo_macos::aac::AudioToolboxAac
        .encode_wav_to_m4a(&wav, Path::new("/tmp/aac/out.m4a"), 96)
        .unwrap();
    std::fs::copy(&wav, "/tmp/aac/in.wav").unwrap();
}
