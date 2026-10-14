//! Model lifecycle, job manager and stores (BRIEF §10, §8.5).
use parlo_core::error::{Error, Result};
use parlo_core::jobs::{JobManager, JobState};
use parlo_core::models::*;
use parlo_core::paths::AppPaths;
use parlo_core::platform::{ManualMemoryPressure, MemoryPressureLevel, NoopPowerGuard};
use parlo_core::recorder::sidecar::{RecState, Sidecar};
use parlo_core::store::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

struct Fake(Arc<AtomicUsize>);
impl Drop for Fake {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn host(timeout: IdleTimeout) -> (Arc<ModelHost>, Arc<Mutex<Vec<ModelsEvent>>>) {
    let ev = Arc::new(Mutex::new(Vec::new()));
    let e2 = ev.clone();
    (
        ModelHost::new(timeout, Arc::new(move |e| e2.lock().unwrap().push(e))),
        ev,
    )
}

fn slot(h: &ModelHost, live: &Arc<AtomicUsize>, loads: &Arc<AtomicUsize>) -> Arc<Slot<Fake>> {
    let (live, loads) = (live.clone(), loads.clone());
    h.slot(EngineKind::Stt, move || {
        loads.fetch_add(1, Ordering::SeqCst);
        live.fetch_add(1, Ordering::SeqCst);
        Ok(Fake(live.clone()))
    })
}

fn wait_for(what: &str, ms: u64, mut f: impl FnMut() -> bool) {
    let t = std::time::Instant::now();
    while t.elapsed() < Duration::from_millis(ms) {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out: {what}");
}

#[test]
fn nothing_loads_until_first_use_then_unloads_after_the_idle_timeout() {
    let (h, ev) = host(IdleTimeout::After(Duration::from_millis(300)));
    let (live, loads) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let s = slot(&h, &live, &loads);
    assert_eq!(live.load(Ordering::SeqCst), 0, "nothing loaded at startup");
    {
        let _g = s.acquire().unwrap();
        assert_eq!(live.load(Ordering::SeqCst), 1);
        std::thread::sleep(Duration::from_millis(500));
        assert_eq!(
            live.load(Ordering::SeqCst),
            1,
            "never unloaded while a job holds it"
        );
    }
    assert!(
        h.idle_unload_in(EngineKind::Stt).is_some(),
        "timer armed on release"
    );
    wait_for("idle unload", 2000, || live.load(Ordering::SeqCst) == 0);
    let states: Vec<ModelState> = ev.lock().unwrap().iter().map(|e| e.state).collect();
    assert_eq!(states.first(), Some(&ModelState::Loading));
    assert!(
        states.contains(&ModelState::Ready) && states.last() == Some(&ModelState::Unloaded),
        "{states:?}"
    );
    // next use reloads
    let _g = s.acquire().unwrap();
    assert_eq!(loads.load(Ordering::SeqCst), 2);
}

#[test]
fn reuse_within_the_timeout_rearms_the_single_timer_and_does_not_reload() {
    let (h, _ev) = host(IdleTimeout::After(Duration::from_millis(400)));
    let (live, loads) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let s = slot(&h, &live, &loads);
    for _ in 0..3 {
        drop(s.acquire().unwrap());
        std::thread::sleep(Duration::from_millis(250)); // < timeout each time, but 750 ms in total
    }
    assert_eq!(loads.load(Ordering::SeqCst), 1, "re-armed, not reloaded");
    assert_eq!(live.load(Ordering::SeqCst), 1);
    wait_for("unload after the last use", 2000, || {
        live.load(Ordering::SeqCst) == 0
    });
}

#[test]
fn immediately_setting_unloads_on_release_and_pressure_unloads_idle_engines() {
    let (h, _ev) = host(IdleTimeout::Immediately);
    let (live, loads) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let s = slot(&h, &live, &loads);
    drop(s.acquire().unwrap());
    assert_eq!(live.load(Ordering::SeqCst), 0);

    h.set_idle_timeout(IdleTimeout::After(Duration::from_secs(600)));
    let mp = ManualMemoryPressure::new();
    let critical = Arc::new(Mutex::new(Vec::new()));
    let c2 = critical.clone();
    h.watch_memory_pressure(&mp, move |c| c2.lock().unwrap().push(c));
    drop(s.acquire().unwrap());
    assert_eq!(live.load(Ordering::SeqCst), 1, "long timeout: stays loaded");
    mp.send(MemoryPressureLevel::Warning);
    wait_for("warning unloads idle engines", 2000, || {
        live.load(Ordering::SeqCst) == 0
    });
    mp.send(MemoryPressureLevel::Critical);
    mp.send(MemoryPressureLevel::Normal);
    wait_for("critical callback", 2000, || {
        critical.lock().unwrap().len() == 2
    });
    assert_eq!(*critical.lock().unwrap(), vec![true, false]);
}

#[test]
fn a_busy_engine_is_never_unloaded_by_pressure() {
    let (h, _ev) = host(IdleTimeout::After(Duration::from_secs(60)));
    let (live, loads) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let s = slot(&h, &live, &loads);
    let g = s.acquire().unwrap();
    h.unload_all_idle();
    assert_eq!(live.load(Ordering::SeqCst), 1);
    drop(g);
}

#[test]
fn load_failure_is_reported_and_leaves_the_slot_unloaded() {
    let (h, ev) = host(IdleTimeout::DEFAULT);
    let s: Arc<Slot<Fake>> = h.slot(EngineKind::Tts, || -> Result<Fake> {
        Err(Error::Other("model file missing".into()))
    });
    assert!(matches!(s.acquire(), Err(Error::Other(m)) if m.contains("missing")));
    assert!(!s.is_loaded());
    assert_eq!(
        ev.lock().unwrap().last().unwrap().state,
        ModelState::Unloaded
    );
}

#[test]
fn jobs_run_one_at_a_time_per_engine_in_fifo_order_and_stt_tts_run_concurrently() {
    let changes = Arc::new(Mutex::new(Vec::new()));
    let c2 = changes.clone();
    let jm = JobManager::new(
        Arc::new(NoopPowerGuard),
        Arc::new(move |i| c2.lock().unwrap().push((i.id, i.state.clone()))),
    );
    let order = Arc::new(Mutex::new(Vec::new()));
    let concurrent = Arc::new(AtomicUsize::new(0));
    let max_concurrent = Arc::new(AtomicUsize::new(0));
    let mut ids = Vec::new();
    for n in 0..4 {
        let (order, conc, maxc) = (order.clone(), concurrent.clone(), max_concurrent.clone());
        ids.push(jm.submit(EngineKind::Stt, "stt", move |_| {
            let c = conc.fetch_add(1, Ordering::SeqCst) + 1;
            maxc.fetch_max(c, Ordering::SeqCst);
            order.lock().unwrap().push(n);
            std::thread::sleep(Duration::from_millis(30));
            conc.fetch_sub(1, Ordering::SeqCst);
            Ok(())
        }));
    }
    // a TTS job runs while the STT queue is busy
    let tts_ran_during_stt = Arc::new(AtomicUsize::new(0));
    let (t2, conc2) = (tts_ran_during_stt.clone(), concurrent.clone());
    let tts = jm.submit(EngineKind::Tts, "tts", move |_| {
        t2.store(conc2.load(Ordering::SeqCst), Ordering::SeqCst);
        Ok(())
    });
    for id in ids {
        assert_eq!(jm.wait(id), JobState::Done);
    }
    assert_eq!(jm.wait(tts), JobState::Done);
    assert_eq!(*order.lock().unwrap(), vec![0, 1, 2, 3], "FIFO");
    assert_eq!(
        max_concurrent.load(Ordering::SeqCst),
        1,
        "one STT job at a time"
    );
    assert!(changes
        .lock()
        .unwrap()
        .iter()
        .any(|(_, s)| *s == JobState::Queued));
}

#[test]
fn cancel_works_for_queued_and_running_jobs_and_failures_are_visible() {
    let jm = JobManager::new(Arc::new(NoopPowerGuard), Arc::new(|_| {}));
    let started = Arc::new(AtomicUsize::new(0));
    let s2 = started.clone();
    let running = jm.submit(EngineKind::Stt, "long", move |ctx| {
        s2.fetch_add(1, Ordering::SeqCst);
        while !ctx.cancel.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
        Err(Error::Cancelled)
    });
    let queued = jm.submit(EngineKind::Stt, "queued", |_| Ok(()));
    let failing = jm.submit(EngineKind::Tts, "fails", |_| {
        Err(Error::Other("boom".into()))
    });
    let panicking = jm.submit(EngineKind::Tts, "panics", |_| panic!("kaboom"));
    wait_for("running job started", 1000, || {
        started.load(Ordering::SeqCst) == 1
    });
    jm.cancel(queued);
    jm.cancel(running);
    assert_eq!(jm.wait(running), JobState::Cancelled);
    assert_eq!(jm.wait(queued), JobState::Cancelled);
    assert!(matches!(jm.wait(failing), JobState::Failed(m) if m == "boom"));
    assert!(
        matches!(jm.wait(panicking), JobState::Failed(_)),
        "a panicking job doesn't kill the worker"
    );
    let after = jm.submit(EngineKind::Tts, "after", |_| Ok(()));
    assert_eq!(jm.wait(after), JobState::Done);
}

#[test]
fn critical_pressure_pauses_the_queue() {
    let jm = JobManager::new(Arc::new(NoopPowerGuard), Arc::new(|_| {}));
    jm.set_paused(true);
    let id = jm.submit(EngineKind::Stt, "x", |_| Ok(()));
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(jm.info(id).unwrap().state, JobState::Queued);
    jm.set_paused(false);
    assert_eq!(jm.wait(id), JobState::Done);
}

#[test]
fn settings_round_trip_patch_and_validation() {
    let dir = tempfile::tempdir().unwrap();
    let paths = AppPaths::under(dir.path());
    let s = Settings::load(&paths);
    assert_eq!(s, Settings::default());
    let s2 = s.apply_patch(&serde_json::json!({"default_voice": "alba", "model_idle_timeout": "mins5", "default_speed": 1.25})).unwrap();
    s2.save(&paths).unwrap();
    assert_eq!(Settings::load(&paths), s2);
    assert_eq!(
        s2.model_idle_timeout.to_timeout(),
        IdleUnload::Mins5.to_timeout()
    );
    assert!(s.apply_patch(&serde_json::json!({"nope": 1})).is_err());
    assert!(s
        .apply_patch(&serde_json::json!({"default_speed": 5.0}))
        .is_err());
    assert!(s
        .apply_patch(&serde_json::json!({"appearance": "purple"}))
        .is_err());
    // unknown/old files keep working (serde defaults)
    std::fs::write(paths.settings_file(), br#"{"default_voice":"mary"}"#).unwrap();
    assert_eq!(Settings::load(&paths).default_voice, "mary");
    std::fs::write(paths.settings_file(), b"not json").unwrap();
    assert_eq!(
        Settings::load(&paths),
        Settings::default(),
        "a corrupt file falls back to defaults instead of crashing"
    );
}

fn make_recording(paths: &AppPaths, id: &str, transcribed: bool, state: RecState, age_days: u64) {
    let dir = paths.recordings();
    std::fs::create_dir_all(&dir).unwrap();
    let wav = dir.join(format!("{id}.wav"));
    std::fs::write(&wav, b"RIFF").unwrap();
    let t = SystemTime::now() - Duration::from_secs(age_days * 86_400);
    std::fs::File::options()
        .write(true)
        .open(&wav)
        .unwrap()
        .set_modified(t)
        .unwrap();
    Sidecar {
        version: 1,
        id: id.into(),
        file: format!("{id}.wav"),
        state,
        sample_rate: 48_000,
        started_at: "2026-01-01T00:00:00+00:00".into(),
        devices: vec![],
        pauses: vec![],
        gaps: vec![],
        frames_written: 0,
        duration_ms: 0,
        overruns: 0,
        stop_reason: None,
        transcribed,
    }
    .write(&dir)
    .unwrap();
}

#[test]
fn retention_never_deletes_untranscribed_recordings_or_the_current_output() {
    let dir = tempfile::tempdir().unwrap();
    let paths = AppPaths::under(dir.path());
    make_recording(&paths, "rec-old-done", true, RecState::Finalized, 10);
    make_recording(&paths, "rec-old-pending", false, RecState::Finalized, 400);
    make_recording(
        &paths,
        "rec-old-recovered-pending",
        false,
        RecState::Recovered,
        400,
    );
    make_recording(&paths, "rec-new-done", true, RecState::Finalized, 1);
    for (name, age) in [("out-old", 20u64), ("out-current", 20), ("out-new", 1)] {
        let d = paths.tts_outputs().join(name);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("seg-0000.wav"), b"x").unwrap();
        let t = SystemTime::now() - Duration::from_secs(age * 86_400);
        std::fs::File::open(&d).unwrap().set_modified(t).unwrap();
    }
    let ws = Workspace {
        tts: Some(WorkspaceTts {
            text: "t".into(),
            voice: "george".into(),
            output_id: Some("out-current".into()),
            source_name: None,
        }),
        ..Default::default()
    };
    let s = Settings::default(); // 7 days
    let rep = cleanup(&paths, &s, &ws, SystemTime::now()).unwrap();
    assert_eq!(rep.recordings_deleted, vec!["rec-old-done".to_string()]);
    assert!(
        paths.recordings().join("rec-old-pending.wav").exists()
            && paths
                .recordings()
                .join("rec-old-recovered-pending.wav")
                .exists()
    );
    assert!(paths.recordings().join("rec-new-done.wav").exists());
    assert_eq!(rep.outputs_deleted, vec!["out-old".to_string()]);
    assert!(
        paths.tts_outputs().join("out-current").exists()
            && paths.tts_outputs().join("out-new").exists()
    );
    // "Forever" keeps even old transcribed recordings
    make_recording(&paths, "rec-ancient", true, RecState::Finalized, 900);
    let s = s
        .apply_patch(&serde_json::json!({"recording_retention": "forever"}))
        .unwrap();
    assert!(cleanup(&paths, &s, &ws, SystemTime::now())
        .unwrap()
        .recordings_deleted
        .is_empty());
}

#[test]
fn workspace_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let paths = AppPaths::under(dir.path());
    let ws = Workspace {
        transcript: Some(WorkspaceTranscript {
            text: "hello".into(),
            source_name: "a.mp3".into(),
            language: Some("en".into()),
            duration_s: 12.5,
            words: 1,
            edited: true,
            exported_or_copied: false,
            recording_id: None,
        }),
        tts: None,
    };
    ws.save(&paths).unwrap();
    assert_eq!(Workspace::load(&paths), ws);
}
