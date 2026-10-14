//! Recorder pipeline tests (BRIEF §8, §14.1) driven by `FileSource` — the real ring buffer / writer / supervisor.
use crossbeam_channel::{unbounded, Receiver};
use parlo_core::audio::wav::{repair_wav, PcmFile};
use parlo_core::error::Error;
use parlo_core::platform::file_source::{Fault, FileSource, Mode, COUNTER_MOD};
use parlo_core::platform::*;
use parlo_core::recorder::sidecar::{GapReason, RecState, Sidecar};
use parlo_core::recorder::{
    recovery, start, RecorderConfig, RecorderDeps, RecorderState, RecorderWarning,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct FakeDisk(AtomicU64);
impl DiskSpace for FakeDisk {
    fn free_bytes(&self, _p: &std::path::Path) -> std::io::Result<u64> {
        Ok(self.0.load(Ordering::SeqCst))
    }
}

struct Rig {
    src: Arc<FileSource>,
    sw: Arc<ManualSleepWake>,
    disk: Arc<FakeDisk>,
    dir: tempfile::TempDir,
    events: Receiver<RecorderWarning>,
    tx: crossbeam_channel::Sender<RecorderWarning>,
}
impl Rig {
    fn new(src: FileSource) -> Self {
        let (tx, events) = unbounded();
        Self {
            src: Arc::new(src),
            sw: Arc::new(ManualSleepWake::new()),
            disk: Arc::new(FakeDisk(AtomicU64::new(100_000_000_000))),
            dir: tempfile::tempdir().unwrap(),
            events,
            tx,
        }
    }
    fn deps(&self) -> RecorderDeps {
        RecorderDeps {
            input: self.src.clone(),
            power: Arc::new(NoopPowerGuard),
            sleep_wake: self.sw.clone(),
            disk: self.disk.clone(),
        }
    }
    fn cfg(&self) -> RecorderConfig {
        let mut c = RecorderConfig::new(self.dir.path());
        c.default_poll_interval = Duration::from_millis(100);
        c.stall_after = Duration::from_millis(600);
        c.backoff = vec![Duration::from_millis(20), Duration::from_millis(50)];
        c.sync_interval = Duration::from_millis(500);
        c
    }
    fn warnings(&self) -> Vec<RecorderWarning> {
        self.events.try_iter().collect()
    }
}

fn wait_until(what: &str, timeout: Duration, mut f: impl FnMut() -> bool) {
    let t = Instant::now();
    while t.elapsed() < timeout {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out waiting for: {what}");
}

fn read_all(path: &std::path::Path) -> (u32, Vec<i16>) {
    let mut r = hound::WavReader::open(path).unwrap();
    (
        r.spec().sample_rate,
        r.samples::<i16>().map(|s| s.unwrap()).collect(),
    )
}

fn counter_value(frame: u64) -> i16 {
    (frame % COUNTER_MOD + 1) as i16
}

#[test]
fn accelerated_recording_is_bit_exact_and_ordered() {
    let total = 48_000 * 20u64;
    let rig = Rig::new(FileSource::counter(48_000, Mode::Accelerated).with_total_frames(total));
    let h = start(rig.cfg(), rig.deps(), rig.tx.clone()).unwrap();
    wait_until("20 s of audio written", Duration::from_secs(30), || {
        h.snapshot().frames >= total
    });
    let info = h.stop().unwrap();
    assert_eq!(info.sidecar.state, RecState::Finalized);
    assert_eq!(info.sidecar.frames_written, total);
    assert_eq!(info.sidecar.overruns, 0);
    let (rate, samples) = read_all(&info.wav);
    assert_eq!(rate, 48_000);
    assert_eq!(samples.len() as u64, total, "exact frame count");
    for (i, s) in samples.iter().enumerate() {
        assert_eq!(
            *s,
            counter_value(i as u64),
            "sample {i} (dropped/reordered sample?)"
        );
    }
    assert!(rig.warnings().is_empty());
}

#[test]
fn realtime_recording_matches_wall_clock_without_overruns() {
    let rig = Rig::new(FileSource::counter(48_000, Mode::Realtime));
    let t = Instant::now();
    let h = start(rig.cfg(), rig.deps(), rig.tx.clone()).unwrap();
    std::thread::sleep(Duration::from_secs(3));
    let info = h.stop().unwrap();
    let wall = t.elapsed().as_secs_f64();
    let rec = info.sidecar.frames_written as f64 / 48_000.0;
    assert!(
        (rec - wall).abs() < 0.25,
        "recorded {rec:.3}s vs wall {wall:.3}s"
    );
    assert_eq!(info.sidecar.overruns, 0);
}

#[test]
fn stereo_input_is_downmixed_by_averaging() {
    use parlo_core::platform::file_source::ConstSignal;
    let src = FileSource::new(Box::new(ConstSignal(0.5)), 16_000, 2, Mode::Accelerated)
        .with_total_frames(16_000);
    let rig = Rig::new(src);
    let h = start(rig.cfg(), rig.deps(), rig.tx.clone()).unwrap();
    wait_until("1 s", Duration::from_secs(10), || {
        h.snapshot().frames >= 16_000
    });
    let info = h.stop().unwrap();
    let (_, s) = read_all(&info.wav);
    assert_eq!(s.len(), 16_000);
    assert!(s.iter().all(|v| (*v as i32 - 16384).abs() <= 1));
}

#[test]
fn stream_error_makes_a_gap_marker_and_recording_continues() {
    let skip = 4_800; // 100 ms lost while the stream was down
    let src = FileSource::counter(48_000, Mode::Accelerated)
        .with_total_frames(48_000 * 6)
        .with_faults(vec![Fault::StreamError {
            at_frame: 48_000 * 2,
            skip_frames_on_reopen: skip,
        }]);
    let rig = Rig::new(src);
    let h = start(rig.cfg(), rig.deps(), rig.tx.clone()).unwrap();
    wait_until("source consumed", Duration::from_secs(30), || {
        rig.src.position() >= 48_000 * 6
    });
    std::thread::sleep(Duration::from_millis(300));
    let info = h.stop().unwrap();
    assert_eq!(rig.src.opens(), 2, "stream was rebuilt once");
    assert_eq!(info.sidecar.gaps.len(), 1);
    assert_eq!(info.sidecar.gaps[0].reason, GapReason::StreamError);
    assert_eq!(info.sidecar.gaps[0].at_frame, 48_000 * 2);
    let (_, s) = read_all(&info.wav);
    assert_eq!(s.len() as u64, 48_000 * 6 - skip);
    for (i, v) in s.iter().enumerate() {
        let src_frame = if (i as u64) < 48_000 * 2 {
            i as u64
        } else {
            i as u64 + skip
        };
        assert_eq!(*v, counter_value(src_frame), "frame {i}");
    }
    assert!(rig
        .warnings()
        .iter()
        .any(|w| matches!(w, RecorderWarning::StreamRestarted { .. })));
}

#[test]
fn default_device_switch_to_a_different_sample_rate_keeps_one_file_and_marks_a_gap() {
    // AirPods connect: system default becomes a 16 kHz device. File stays at the first device's rate (48 kHz).
    let src =
        FileSource::counter(48_000, Mode::Realtime).with_faults(vec![Fault::DefaultDeviceChange {
            at_frame: 48_000 * 2,
            new_id: "airpods".into(),
            new_rate: 16_000,
        }]);
    let rig = Rig::new(src);
    let h = start(rig.cfg(), rig.deps(), rig.tx.clone()).unwrap();
    std::thread::sleep(Duration::from_secs(5));
    let info = h.stop().unwrap();
    assert_eq!(info.sidecar.sample_rate, 48_000);
    assert_eq!(info.sidecar.devices.len(), 2, "{:?}", info.sidecar.devices);
    assert_eq!(info.sidecar.devices[1].name, "airpods");
    assert_eq!(info.sidecar.devices[1].sample_rate, 16_000);
    assert_eq!(info.sidecar.gaps.len(), 1);
    assert_eq!(info.sidecar.gaps[0].reason, GapReason::DeviceChanged);
    assert!(
        info.sidecar.gaps[0].gap_ms < 1500,
        "switch took {} ms",
        info.sidecar.gaps[0].gap_ms
    );
    let (rate, s) = read_all(&info.wav);
    assert_eq!(rate, 48_000);
    // ~5 s of audio at 48 kHz regardless of the 16 kHz leg (resampled), minus the switch gap
    let secs = s.len() as f64 / 48_000.0;
    assert!((4.0..5.2).contains(&secs), "{secs}");
    assert!(rig
        .warnings()
        .iter()
        .any(|w| matches!(w, RecorderWarning::DeviceChanged { .. })));
}

#[test]
fn device_removed_falls_back_to_new_default() {
    let src = FileSource::counter(48_000, Mode::Realtime).with_faults(vec![Fault::DeviceRemoved {
        at_frame: 48_000,
        new_id: "builtin".into(),
        new_rate: 44_100,
    }]);
    let rig = Rig::new(src);
    let h = start(rig.cfg(), rig.deps(), rig.tx.clone()).unwrap();
    std::thread::sleep(Duration::from_secs(3));
    let info = h.stop().unwrap();
    assert_eq!(info.sidecar.gaps.len(), 1);
    assert_eq!(info.sidecar.gaps[0].reason, GapReason::DeviceLost);
    assert_eq!(info.sidecar.devices.last().unwrap().name, "builtin");
}

#[test]
fn stalled_input_is_rebuilt_after_a_second() {
    let src = FileSource::counter(48_000, Mode::Realtime).with_faults(vec![Fault::Stall {
        at_frame: 48_000,
        dur: Duration::from_millis(2500),
    }]);
    let rig = Rig::new(src);
    let h = start(rig.cfg(), rig.deps(), rig.tx.clone()).unwrap();
    std::thread::sleep(Duration::from_secs(5));
    let info = h.stop().unwrap();
    assert!(
        info.sidecar
            .gaps
            .iter()
            .any(|g| g.reason == GapReason::Stall),
        "{:?}",
        info.sidecar.gaps
    );
    assert!(rig.src.opens() >= 2);
}

#[test]
fn sleep_and_wake_flush_then_resume_with_a_gap() {
    let rig = Rig::new(FileSource::counter(48_000, Mode::Realtime));
    let h = start(rig.cfg(), rig.deps(), rig.tx.clone()).unwrap();
    std::thread::sleep(Duration::from_millis(800));
    rig.sw.send(PowerEvent::WillSleep);
    wait_until("stream closed", Duration::from_secs(5), || {
        rig.src.opens() == 1
    });
    let frames_at_sleep = {
        std::thread::sleep(Duration::from_millis(300));
        h.snapshot().frames
    };
    // while asleep nothing is recorded
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(h.snapshot().frames, frames_at_sleep);
    // the sidecar on disk reflects a flush at sleep time
    let sc = Sidecar::read(&rig.dir.path().join(format!("{}.json", h.id()))).unwrap();
    assert!(sc.frames_written > 0);
    rig.sw.send(PowerEvent::DidWake);
    std::thread::sleep(Duration::from_millis(1200));
    let info = h.stop().unwrap();
    assert_eq!(info.sidecar.gaps.len(), 1);
    assert_eq!(info.sidecar.gaps[0].reason, GapReason::ResumedAfterSleep);
    assert!(
        info.sidecar.gaps[0].gap_ms >= 700,
        "gap {} ms",
        info.sidecar.gaps[0].gap_ms
    );
    assert!(rig
        .warnings()
        .iter()
        .any(|w| matches!(w, RecorderWarning::ResumedAfterSleep { .. })));
    assert!(info.sidecar.frames_written as f64 / 48_000.0 > 1.6);
}

#[test]
fn pause_and_resume_do_not_record_in_between() {
    let rig = Rig::new(FileSource::counter(48_000, Mode::Realtime));
    let h = start(rig.cfg(), rig.deps(), rig.tx.clone()).unwrap();
    std::thread::sleep(Duration::from_millis(800));
    h.pause().unwrap();
    assert_eq!(h.snapshot().state, RecorderState::Paused);
    let f0 = h.snapshot().frames;
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(h.snapshot().frames, f0, "no audio while paused");
    h.resume().unwrap();
    std::thread::sleep(Duration::from_millis(800));
    let info = h.stop().unwrap();
    assert_eq!(info.sidecar.pauses.len(), 1);
    assert_eq!(info.sidecar.pauses[0].start_frame, f0);
    assert_eq!(info.sidecar.pauses[0].end_frame, Some(f0));
    let secs = info.sidecar.frames_written as f64 / 48_000.0;
    assert!((1.3..2.0).contains(&secs), "{secs}");
}

#[test]
fn denied_mic_permission_fails_cleanly_without_files() {
    let rig = Rig::new(FileSource::counter(48_000, Mode::Accelerated).deny_permission());
    let r = start(rig.cfg(), rig.deps(), rig.tx.clone());
    assert!(matches!(r, Err(Error::MicPermission)));
    assert_eq!(
        std::fs::read_dir(rig.dir.path()).unwrap().count(),
        0,
        "no partial files"
    );
}

#[test]
fn disk_preflight_warning_and_graceful_stop() {
    // pre-flight refuses below 300 MB
    let rig = Rig::new(FileSource::counter(48_000, Mode::Realtime));
    rig.disk.0.store(100_000_000, Ordering::SeqCst);
    assert!(matches!(
        start(rig.cfg(), rig.deps(), rig.tx.clone()),
        Err(Error::DiskSpace { .. })
    ));
    // warn below 2 GB, then stop gracefully (keeping the file) below 300 MB
    rig.disk.0.store(1_500_000_000, Ordering::SeqCst);
    let mut cfg = rig.cfg();
    cfg.disk_check_interval = Duration::from_millis(100);
    let h = start(cfg, rig.deps(), rig.tx.clone()).unwrap();
    assert!(rig
        .warnings()
        .iter()
        .any(|w| matches!(w, RecorderWarning::LowDisk { .. })));
    std::thread::sleep(Duration::from_millis(700));
    rig.disk.0.store(200_000_000, Ordering::SeqCst);
    wait_until("graceful stop", Duration::from_secs(5), || {
        h.snapshot().state == RecorderState::Stopped
    });
    assert!(rig
        .warnings()
        .iter()
        .any(|w| matches!(w, RecorderWarning::Stopped { .. })));
    let info = h.stop().unwrap();
    assert_eq!(info.sidecar.state, RecState::StoppedByError);
    let (_, s) = read_all(&info.wav);
    assert!(s.len() > 20_000, "audio recorded before the stop is kept");
}

#[test]
fn pinned_device_that_vanishes_falls_back_to_default() {
    let src = FileSource::counter(48_000, Mode::Realtime).with_devices(vec![
        DeviceInfo {
            id: "file-default".into(),
            name: "Built-in".into(),
            is_default: true,
        },
        DeviceInfo {
            id: "usb".into(),
            name: "USB mic".into(),
            is_default: false,
        },
    ]);
    let rig = Rig::new(src);
    let mut cfg = rig.cfg();
    cfg.device = DeviceSelector::Named("usb".into());
    let h = start(cfg, rig.deps(), rig.tx.clone()).unwrap();
    std::thread::sleep(Duration::from_millis(600));
    // unplug it: the next sleep/wake forces a reopen of the pinned device, which now fails
    rig.src.remove_device("usb");
    rig.sw.send(PowerEvent::WillSleep);
    std::thread::sleep(Duration::from_millis(200));
    rig.sw.send(PowerEvent::DidWake);
    std::thread::sleep(Duration::from_millis(1000));
    let info = h.stop().unwrap();
    assert!(rig
        .warnings()
        .iter()
        .any(|w| matches!(w, RecorderWarning::PinnedDeviceLost { .. })));
    assert_eq!(info.sidecar.devices.last().unwrap().name, "file-default");
}

#[test]
fn silent_input_warning_after_threshold() {
    use parlo_core::platform::file_source::ConstSignal;
    let rig = Rig::new(FileSource::new(
        Box::new(ConstSignal(0.0)),
        16_000,
        1,
        Mode::Realtime,
    ));
    let mut cfg = rig.cfg();
    cfg.silent_after = Duration::from_millis(800);
    let h = start(cfg, rig.deps(), rig.tx.clone()).unwrap();
    std::thread::sleep(Duration::from_millis(1800));
    let _ = h.stop().unwrap();
    assert!(rig
        .warnings()
        .iter()
        .any(|w| matches!(w, RecorderWarning::SilentInput)));
}

#[test]
fn discard_deletes_the_recording() {
    let rig = Rig::new(FileSource::counter(16_000, Mode::Realtime));
    let h = start(rig.cfg(), rig.deps(), rig.tx.clone()).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    h.discard().unwrap();
    assert_eq!(std::fs::read_dir(rig.dir.path()).unwrap().count(), 0);
}

#[test]
fn recovery_repairs_a_truncated_recording() {
    // simulate a crash: sidecar still says `recording`, WAV header has stale sizes and a torn trailing frame
    let rig =
        Rig::new(FileSource::counter(48_000, Mode::Accelerated).with_total_frames(48_000 * 3));
    let h = start(rig.cfg(), rig.deps(), rig.tx.clone()).unwrap();
    wait_until("3 s", Duration::from_secs(10), || {
        h.snapshot().frames >= 48_000 * 3
    });
    let info = h.stop().unwrap();
    // rewrite the sidecar as if the app died mid-recording, truncate the WAV mid-frame, corrupt header sizes
    let mut sc = info.sidecar.clone();
    sc.state = RecState::Recording;
    sc.write(rig.dir.path()).unwrap();
    let f = std::fs::OpenOptions::new()
        .write(true)
        .open(&info.wav)
        .unwrap();
    let len = f.metadata().unwrap().len();
    f.set_len(len - 3).unwrap(); // torn frame
    use std::os::unix::fs::FileExt;
    f.write_all_at(&0u32.to_le_bytes(), 40).unwrap(); // stale data size
    drop(f);
    let rec = recovery::recover_all(rig.dir.path()).unwrap();
    assert_eq!(rec.len(), 1);
    assert_eq!(rec[0].sidecar.state, RecState::Recovered);
    let pf = PcmFile::open(&rec[0].wav).unwrap();
    assert!(
        pf.frames >= 48_000 * 3 - 2 && pf.frames <= 48_000 * 3,
        "{}",
        pf.frames
    );
    let (_, s) = read_all(&rec[0].wav);
    assert_eq!(s.len() as u64, pf.frames);
    assert_eq!(
        repair_wav(&rec[0].wav).unwrap().frames,
        pf.frames,
        "repair is idempotent"
    );
}

#[test]
fn soak_three_hours_accelerated_has_exact_frame_count_and_bounded_memory() {
    fn rss_mb() -> f64 {
        let s = std::fs::read_to_string("/proc/self/status").unwrap();
        s.lines()
            .find_map(|l| l.strip_prefix("VmRSS:"))
            .map(|v| {
                v.trim()
                    .trim_end_matches(" kB")
                    .trim()
                    .parse::<f64>()
                    .unwrap()
                    / 1024.0
            })
            .unwrap()
    }
    let total = 3 * 3600 * 48_000u64;
    let rig = Rig::new(FileSource::counter(48_000, Mode::Accelerated).with_total_frames(total));
    let rss0 = rss_mb();
    let mut peak = rss0;
    let t0 = Instant::now();
    let h = start(rig.cfg(), rig.deps(), rig.tx.clone()).unwrap();
    while h.snapshot().frames < total {
        std::thread::sleep(Duration::from_millis(250));
        peak = peak.max(rss_mb());
    }
    let info = h.stop().unwrap();
    assert_eq!(info.sidecar.frames_written, total, "exact frame count");
    assert_eq!(info.sidecar.overruns, 0);
    assert!(
        peak - rss0 < 64.0,
        "RSS grew by {:.1} MB during a 3 h recording",
        peak - rss0
    );
    // valid WAV of the right size, header consistent
    let pf = PcmFile::open(&info.wav).unwrap();
    assert_eq!(pf.frames, total);
    let r = hound::WavReader::open(&info.wav).unwrap();
    assert_eq!(r.duration() as u64, total);
    // spot-check content at several offsets (order + no drops)
    let mut buf = Vec::new();
    for start_frame in [0u64, 1_234_567, total / 2, total - 5000] {
        pf.read_f32(start_frame, 4000, &mut buf).unwrap();
        for (i, v) in buf.iter().enumerate() {
            assert_eq!(
                (v * 32768.0).round() as i64,
                counter_value(start_frame + i as u64) as i64,
                "frame {}",
                start_frame + i as u64
            );
        }
    }
    eprintln!(
        "soak: 3 h of audio recorded in {:.1}s wall, RSS +{:.1} MB",
        t0.elapsed().as_secs_f64(),
        peak - rss0
    );
}
