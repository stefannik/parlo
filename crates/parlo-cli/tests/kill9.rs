//! BRIEF §14.1: `kill -9` mid-recording → header repair yields a playable file with ≤ 2 s loss (+ one sync interval).
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn kill_nine_mid_recording_loses_at_most_the_sync_interval() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_parlo-cli"))
        .args(["record", "--dir"])
        .arg(dir.path())
        .args(["--source", "counter", "--rate", "48000", "--seconds", "120"])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    // wait for the "recording started" line, then record for a while in real time
    let mut line = String::new();
    BufReader::new(child.stdout.as_mut().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert!(line.contains("recording"), "{line}");
    let t = Instant::now();
    std::thread::sleep(Duration::from_millis(7300));
    unsafe { libc::kill(child.id() as i32, libc::SIGKILL) };
    let killed_after = t.elapsed().as_secs_f64();
    child.wait().unwrap();

    // recover
    let out = Command::new(env!("CARGO_BIN_EXE_parlo-cli"))
        .args(["recover"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let info: serde_json::Value =
        serde_json::from_slice(out.stdout.split(|b| *b == b'\n').next().unwrap()).unwrap();
    let wav = info["wav"].as_str().unwrap();
    let mut r = hound::WavReader::open(wav).expect("recovered file must be a valid WAV");
    assert_eq!(r.spec().sample_rate, 48_000);
    let frames = r.duration() as f64;
    let lost = killed_after - frames / 48_000.0;
    eprintln!(
        "killed after {killed_after:.2}s; recovered {:.2}s; lost {lost:.2}s",
        frames / 48_000.0
    );
    assert!(
        lost <= 2.6,
        "lost {lost:.2}s (> 2 s sync interval + scheduling slack)"
    );
    assert!(
        lost >= -0.2,
        "recovered more audio than was recorded?! {lost}"
    );
    // content is intact and ordered up to the cut
    let samples: Vec<i16> = r.samples::<i16>().map(|s| s.unwrap()).collect();
    for (i, s) in samples.iter().enumerate() {
        assert_eq!(*s as u64, i as u64 % 30_000 + 1, "frame {i}");
    }
    // sidecar now says recovered
    let sc: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            dir.path()
                .join(format!("{}.json", info["id"].as_str().unwrap())),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(sc["state"], "recovered");
}
