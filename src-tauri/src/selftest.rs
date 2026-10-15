//! `PARLO_SELFTEST=1`: scripted end-to-end run of the IPC command layer with the REAL engines inside the real Tauri
//! process (used on Linux under Xvfb by `scripts/selftest-linux.sh`). It calls the very same command functions the UI
//! reaches over IPC, with `Channel`s that collect the events, so command routing, DTO serialization, job management,
//! model lifecycle, caching, export and storage handling are exercised without a human or a browser driver.
//! Prints one line per step and `PARLO_SELFTEST_OK`, or `PARLO_SELFTEST_FAIL: <why>` (exit 1).

use crate::commands as c;
use crate::ipc::*;
use crate::state::AppState;
use std::sync::{mpsc, Arc};
use std::time::Duration;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Manager};

type R<T> = Result<T, String>;

fn channel<E: serde::de::DeserializeOwned + Send + 'static>() -> (Channel<E>, mpsc::Receiver<E>) {
    let (tx, rx) = mpsc::channel();
    let ch = Channel::new(move |body| {
        if let InvokeResponseBody::Json(s) = body {
            if let Ok(e) = serde_json::from_str::<E>(&s) {
                let _ = tx.send(e);
            }
        }
        Ok(())
    });
    (ch, rx)
}

fn ensure(cond: bool, what: impl Into<String>) -> R<()> {
    if cond {
        Ok(())
    } else {
        Err(what.into())
    }
}

fn ipc<T>(r: IpcResult<T>, what: &str) -> R<T> {
    r.map_err(|e| format!("{what}: [{}] {}", e.code, e.message))
}

fn drain_tts(
    rx: &mpsc::Receiver<TtsEventDto>,
    stop_after_first_segment: Option<&dyn Fn()>,
) -> R<Vec<TtsEventDto>> {
    let mut out = Vec::new();
    let mut fired = false;
    loop {
        let e = rx.recv_timeout(Duration::from_secs(900)).map_err(|e| {
            format!("TTS event stream ended without a terminal event ({e}); got {out:?}")
        })?;
        let end = matches!(e, TtsEventDto::Done { .. } | TtsEventDto::Error { .. });
        if let (TtsEventDto::SegmentReady { .. }, Some(f)) = (&e, stop_after_first_segment) {
            if !fired {
                fired = true;
                f();
            }
        }
        out.push(e);
        if end {
            return Ok(out);
        }
    }
}

fn script(app: &AppHandle, state: &Arc<AppState>, say: &dyn Fn(&str)) -> R<()> {
    let st = || app.state::<Arc<AppState>>();
    let tmp = std::env::temp_dir().join(format!("parlo-selftest-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;

    // --- settings -------------------------------------------------------------------------------------------------
    let blank = SettingsPatch::default();
    let s = ipc(
        c::settings_update(
            st(),
            SettingsPatch {
                appearance: Some(Appearance::Dark),
                default_speed: Some(1.25),
                ..blank.clone()
            },
        ),
        "settings_update",
    )?;
    ensure(
        s.appearance == Appearance::Dark && (s.default_speed - 1.25).abs() < 1e-6,
        "settings patch not applied",
    )?;
    ensure(
        c::settings_update(
            st(),
            SettingsPatch {
                default_speed: Some(9.0),
                ..blank.clone()
            },
        )
        .is_err(),
        "out-of-range speed accepted",
    )?;
    ensure(
        c::settings_get(st()).appearance == Appearance::Dark,
        "settings not persisted in memory",
    )?;
    ipc(
        c::settings_update(
            st(),
            SettingsPatch {
                input_device: Some("some-device".into()),
                ..blank.clone()
            },
        ),
        "pin device",
    )?;
    let s = ipc(
        c::settings_update(
            st(),
            SettingsPatch {
                input_device: Some(String::new()),
                ..blank.clone()
            },
        ),
        "unpin device",
    )?;
    ensure(
        s.input_device.is_none(),
        "empty string must reset the input device to the system default",
    )?;
    say("settings: patch, validation, device pin/unpin");

    // --- voices + workspace ---------------------------------------------------------------------------------------
    let voices = c::voices_list(st());
    ensure(
        voices.len() == 10
            && voices
                .iter()
                .any(|v| v.id == "george" && v.label == "George"),
        format!(
            "voices: {:?}",
            voices.iter().map(|v| &v.id).collect::<Vec<_>>()
        ),
    )?;
    ensure(
        voices.iter().all(|v| {
            v.preview_path
                .as_deref()
                .is_none_or(|p| std::path::Path::new(p).exists())
        }),
        "a preview path does not exist",
    )?;
    ipc(
        c::workspace_update(
            st(),
            WorkspacePatch {
                tts: Some(WorkspaceTtsDto {
                    text: "draft".into(),
                    voice: "mary".into(),
                    output_id: None,
                    source_name: None,
                    segments: vec![],
                    duration_ms: None,
                }),
                ..Default::default()
            },
        ),
        "workspace_update",
    )?;
    ensure(
        c::workspace_get(app.clone(), st())
            .tts
            .is_some_and(|t| t.text == "draft" && t.voice == "mary"),
        "workspace round-trip",
    )?;
    ipc(
        c::workspace_update(
            st(),
            WorkspacePatch {
                clear_tts: true,
                ..Default::default()
            },
        ),
        "workspace clear",
    )?;
    ensure(
        c::workspace_get(app.clone(), st()).tts.is_none(),
        "workspace clear",
    )?;
    say("voices (10, incl. George) and workspace round-trip");

    // --- text import ----------------------------------------------------------------------------------------------
    let doc = std::path::Path::new("fixtures/text").join("headers_hyphens.pdf");
    if doc.exists() {
        let r = c::text_import(st(), doc.display().to_string());
        if cfg!(target_os = "macos") {
            // PDFKit extracts and the portable cleanup removes headers/footers/hyphenation
            let r = r.map_err(|e| format!("PDF import (PDFKit): {e:?}"))?;
            ensure(
                r.preview.contains("financial results") && !r.preview.contains("Page 1 of 4"),
                "PDF import via PDFKit gave unexpected text",
            )?;
        } else {
            // no PDF extractor on Linux: must be a clear error, not a crash
            ensure(
                r.is_err(),
                "PDF import without PDFKit should report a clear error on Linux",
            )?;
        }
    }
    let md = tmp.join("notes.md");
    std::fs::write(
        &md,
        "# Title\n\nFirst paragraph of the notes.\n\n- item one\n- item two\n",
    )
    .map_err(|e| e.to_string())?;
    let imp = ipc(
        c::text_import(st(), md.display().to_string()),
        "text_import",
    )?;
    ensure(
        imp.words >= 8 && !imp.preview.is_empty() && imp.file_name == "notes.md",
        format!("import: {imp:?}"),
    )?;
    let full = ipc(
        c::text_import_full(st(), imp.import_id.clone()),
        "text_import_full",
    )?;
    ensure(full.contains("First paragraph"), "import_full text")?;
    say("text import (md) + clear PDF error");

    // --- TTS: generate, cache, export -----------------------------------------------------------------------------
    let req = |text: &str| TtsRequest {
        source: TtsSource::Text { text: text.into() },
        voice: "george".into(),
        quality: "fast".into(),
    };
    let (ch, rx) = channel::<TtsEventDto>();
    ipc(
        c::tts_start(
            app.clone(),
            st(),
            req("Hello from the Parlo self test. This is a short sentence."),
            ch,
        ),
        "tts_start",
    )?;
    let ev = drain_tts(&rx, None)?;
    let (seg_idx, done_idx) = (
        ev.iter()
            .position(|e| matches!(e, TtsEventDto::SegmentReady { .. })),
        ev.iter()
            .position(|e| matches!(e, TtsEventDto::Done { .. })),
    );
    ensure(
        seg_idx.is_some() && done_idx.is_some() && seg_idx < done_idx,
        format!("TTS events out of order: {ev:?}"),
    )?;
    let Some(TtsEventDto::Done {
        output_id,
        duration_ms,
        segments,
        cache_hit,
    }) = ev.last().cloned()
    else {
        return Err(format!("TTS did not finish: {:?}", ev.last()));
    };
    ensure(
        !cache_hit
            && duration_ms > 1000
            && !segments.is_empty()
            && segments.iter().all(|p| std::path::Path::new(p).exists()),
        "TTS result",
    )?;
    say(&format!(
        "tts: {} ms of audio, {} segment(s), first segment before done",
        duration_ms,
        segments.len()
    ));
    let (ch, rx) = channel::<TtsEventDto>();
    let t0 = std::time::Instant::now();
    ipc(
        c::tts_start(
            app.clone(),
            st(),
            req("Hello from the Parlo self test. This is a short sentence."),
            ch,
        ),
        "tts_start (again)",
    )?;
    let ev2 = drain_tts(&rx, None)?;
    ensure(
        matches!(
            ev2.last(),
            Some(TtsEventDto::Done {
                cache_hit: true,
                ..
            })
        ),
        format!("identical request must be a cache hit: {:?}", ev2.last()),
    )?;
    say(&format!(
        "tts: identical request is a cache hit ({} ms)",
        t0.elapsed().as_millis()
    ));
    for (fmt, ext) in [
        (AudioExportFormat::Wav, "wav"),
        (AudioExportFormat::Mp3, "mp3"),
    ] {
        let dest = tmp.join(format!("out.{ext}"));
        let written = ipc(
            tauri::async_runtime::block_on(c::export_tts(
                st(),
                output_id.clone(),
                fmt,
                dest.display().to_string(),
            )),
            "export_tts",
        )?;
        ensure(
            std::fs::metadata(&written)
                .map(|m| m.len() > 1000)
                .unwrap_or(false),
            format!("{ext} export is empty"),
        )?;
    }
    let m4a = tauri::async_runtime::block_on(c::export_tts(
        st(),
        output_id.clone(),
        AudioExportFormat::M4a,
        tmp.join("out.m4a").display().to_string(),
    ));
    if cfg!(target_os = "linux") {
        ensure(
            m4a.is_err(),
            "M4A export must report 'unavailable' on Linux",
        )?;
        say("tts: WAV + MP3 export (M4A unavailable on Linux, reported cleanly)");
    } else {
        m4a.map_err(|e| format!("M4A export: {e:?}"))?;
        ensure(
            std::fs::metadata(tmp.join("out.m4a"))
                .map(|m| m.len() > 1000)
                .unwrap_or(false),
            "m4a export is empty",
        )?;
        say("tts: WAV + MP3 + M4A export");
    }
    // export refuses to overwrite system locations / invalid destinations
    ensure(
        tauri::async_runtime::block_on(c::export_tts(
            st(),
            output_id.clone(),
            AudioExportFormat::Wav,
            "relative/path.wav".into(),
        ))
        .is_err(),
        "relative export destination accepted",
    )?;

    // --- TTS cancel -----------------------------------------------------------------------------------------------
    let long = "This sentence is repeated to make a long document. ".repeat(400);
    let (ch, rx) = channel::<TtsEventDto>();
    let id = ipc(
        c::tts_start(app.clone(), st(), req(&long), ch),
        "tts_start (long)",
    )?;
    let cancelled_at = std::cell::Cell::new(None);
    let cancel = || {
        cancelled_at.set(Some(std::time::Instant::now()));
        c::job_cancel(st(), id)
    };
    let ev = drain_tts(&rx, Some(&cancel))?;
    let latency = cancelled_at.get().map(|t| t.elapsed());
    ensure(
        matches!(ev.last(), Some(TtsEventDto::Error { error }) if error.code == "cancelled"),
        format!("cancel: {:?}", ev.last()),
    )?;
    let latency = latency.ok_or("cancel was never requested")?;
    ensure(
        latency < Duration::from_secs(10),
        format!("cancel took {} ms to stop the job", latency.as_millis()),
    )?;
    say(&format!(
        "tts: cancelled a long job after its first segment — stopped {} ms after the request",
        latency.as_millis()
    ));

    // --- STT: transcribe a fixture ---------------------------------------------------------------------------------
    let wav = std::path::Path::new("fixtures/generated/synth/en_1.wav");
    ensure(
        wav.exists(),
        "fixture generated/synth/en_1.wav missing (scripts/gen-fixtures.sh)",
    )?;
    let (ch, rx) = channel::<SttEventDto>();
    ipc(
        c::transcribe_start(
            app.clone(),
            st(),
            TranscribeRequest {
                source: SttSource::Path {
                    path: wav.canonicalize().unwrap().display().to_string(),
                },
                language: "auto".into(),
            },
            ch,
        ),
        "transcribe_start",
    )?;
    let mut stages = Vec::new();
    let transcript = loop {
        match rx.recv_timeout(Duration::from_secs(1200)).map_err(|e| {
            format!(
                "STT event stream ended without a terminal event ({e}); stages so far: {stages:?}"
            )
        })? {
            SttEventDto::Stage { stage } => stages.push(stage),
            SttEventDto::Done { transcript } => break transcript,
            SttEventDto::Error { error } => {
                return Err(format!(
                    "transcription failed: [{}] {}",
                    error.code, error.message
                ))
            }
            _ => {}
        }
    };
    ensure(
        transcript.language.as_deref() == Some("en")
            && transcript.text.to_lowercase().contains("weather"),
        format!("transcript: {transcript:?}"),
    )?;
    ensure(
        stages.first().map(String::as_str) == Some("loading_model")
            || stages.iter().any(|s| s == "transcribing"),
        format!("stages: {stages:?}"),
    )?;
    say(&format!(
        "stt: \"{}\" ({} s audio, {:.1} s)",
        transcript.text.trim(),
        transcript.duration_s,
        transcript.processing_s
    ));
    ensure(
        state.stt.is_loaded(),
        "the speech model should still be loaded right after a job (idle unload is timer-based)",
    )?;

    // --- storage ----------------------------------------------------------------------------------------------------
    let usage = ipc(c::storage_usage(st()), "storage_usage")?;
    ensure(
        usage.iter().any(|u| u.kind == "outputs" && u.bytes > 0),
        format!("outputs usage: {usage:?}"),
    )?;
    let usage = ipc(c::storage_clear(st(), "outputs".into()), "storage_clear")?;
    ensure(
        usage.iter().any(|u| u.kind == "outputs" && u.bytes == 0),
        "outputs not cleared",
    )?;
    ensure(
        c::storage_clear(st(), "recordings".into()).is_err(),
        "recordings must never be clearable from the UI command",
    )?;
    ensure(
        ipc(c::recoverables_list(st()), "recoverables_list")?.is_empty(),
        "nothing should be recoverable after clean jobs",
    )?;
    say("storage: usage, clear outputs, recordings protected; nothing left to recover");
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(())
}

/// `PARLO_SELFTEST=mic [PARLO_SELFTEST_SECONDS=8]`: the real microphone path inside the bundled app — TCC permission
/// (shows the system prompt the first time), CoreAudio capture through cpal, the power assertion while recording, the
/// frame count against the wall clock, and that the audio is not digital silence.
fn mic_script(app: &AppHandle, say: &dyn Fn(&str)) -> R<()> {
    let secs: u64 = std::env::var("PARLO_SELFTEST_SECONDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8);
    say(&format!(
        "mic permission before: {:?}",
        c::mic_permission_status(st_of(app))
    ));
    let perm = ipc(
        tauri::async_runtime::block_on(c::mic_permission_request(st_of(app))),
        "mic_permission_request",
    )?;
    ensure(
        matches!(perm, MicPermission::Granted),
        format!("microphone permission not granted: {perm:?}"),
    )?;
    say("mic permission granted");
    let devices = ipc(c::input_devices_list(st_of(app)), "input_devices_list")?;
    say(&format!(
        "input devices: {}",
        devices
            .iter()
            .map(|d| d.name.clone())
            .collect::<Vec<_>>()
            .join(", ")
    ));
    let t0 = std::time::Instant::now();
    let info = ipc(
        c::recording_start(app.clone(), st_of(app), None),
        "recording_start",
    )?;
    say(&format!(
        "recording on {:?} at {} Hz",
        info.device, info.sample_rate
    ));
    std::thread::sleep(Duration::from_secs(2));
    let during = String::from_utf8_lossy(
        &std::process::Command::new("pmset")
            .args(["-g", "assertions"])
            .output()
            .map_err(|e| e.to_string())?
            .stdout,
    )
    .to_string();
    let pid = std::process::id().to_string();
    ensure(
        during
            .lines()
            .any(|l| l.contains(&format!("pid {pid}")) && l.contains("PreventUserIdleSystemSleep")),
        format!("no sleep assertion held while recording:\n{during}"),
    )?;
    say("power assertion held while recording (pmset -g assertions)");
    ipc(
        c::recording_pause(app.clone(), st_of(app)),
        "recording_pause",
    )?;
    std::thread::sleep(Duration::from_secs(1));
    ipc(
        c::recording_resume(app.clone(), st_of(app)),
        "recording_resume",
    )?;
    std::thread::sleep(Duration::from_secs(secs.saturating_sub(3).max(1)));
    let out = ipc(c::recording_stop(app.clone(), st_of(app)), "recording_stop")?;
    let wall_ms = t0.elapsed().as_millis() as i64;
    say(&format!(
        "recorded {} ms (wall {} ms incl. 1 s pause), {} gap marker(s)",
        out.duration_ms,
        wall_ms,
        out.gaps.len()
    ));
    let pcm = parlo_core::audio::wav::PcmFile::open(std::path::Path::new(&out.wav_path))
        .map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    pcm.read_f32(0, pcm.frames as usize, &mut buf)
        .map_err(|e| e.to_string())?;
    let peak = buf.iter().fold(0f32, |m, s| m.max(s.abs()));
    let rms = (buf.iter().map(|s| s * s).sum::<f32>() / buf.len().max(1) as f32).sqrt();
    say(&format!("audio peak {peak:.4}, rms {rms:.5}"));
    // Pause closes the stream, so the file holds the recorded time only: wall minus the 1 s pause (± start-up latency).
    let expect_ms = wall_ms - 1000;
    ensure(
        (out.duration_ms as i64 - expect_ms).abs() < 1500,
        format!(
            "duration {} ms vs expected ≈{expect_ms} ms",
            out.duration_ms
        ),
    )?;
    ensure(
        peak > 0.0,
        "the recording is digital silence (muted mic or no permission?)",
    )?;
    // after the stop the assertion must be released
    std::thread::sleep(Duration::from_millis(500));
    let after = String::from_utf8_lossy(
        &std::process::Command::new("pmset")
            .args(["-g", "assertions"])
            .output()
            .map_err(|e| e.to_string())?
            .stdout,
    )
    .to_string();
    ensure(
        !after
            .lines()
            .any(|l| l.contains(&format!("pid {pid}")) && l.contains("PreventUserIdleSystemSleep")),
        "sleep assertion still held after the stop",
    )?;
    say("power assertion released after stop");
    Ok(())
}

/// `PARLO_SELFTEST=lifecycle`: drives the window/job lifecycle for scripts/measure-idle-memory.sh, which samples memory
/// at each `PHASE …` line: window open → window closed (webview destroyed) → window recreated → a TTS + STT job →
/// idle timeout (`PARLO_IDLE_WAIT_S`, default 70 s). Needs the repo root as cwd for the fixture.
fn lifecycle_script(app: &AppHandle, say: &dyn Fn(&str)) -> R<()> {
    let phase = |name: &str, wait_s: u64| {
        println!("PHASE {name}");
        use std::io::Write;
        let _ = std::io::stdout().flush();
        std::thread::sleep(Duration::from_secs(wait_s));
    };
    if let Ok(only) = std::env::var("PARLO_LC_ONLY") {
        return lc_isolated(app, &only);
    }
    phase("open", 20);
    let w = app
        .get_webview_window("main")
        .ok_or("no main window at launch")?;
    w.close().map_err(|e| e.to_string())?;
    phase("closed", 25);
    ensure(
        app.get_webview_window("main").is_none(),
        "the window still exists after close",
    )?;
    let a2 = app.clone();
    app.run_on_main_thread(move || crate::ensure_main_window(&a2))
        .map_err(|e| e.to_string())?;
    phase("reopened", 15);
    ensure(
        app.get_webview_window("main").is_some(),
        "the window was not recreated",
    )?;
    let (ch, rx) = channel::<TtsEventDto>();
    ipc(
        c::tts_start(
            app.clone(),
            st_of(app),
            TtsRequest {
                source: TtsSource::Text {
                    text: "Hello from the Parlo lifecycle test. This is a short sentence.".into(),
                },
                voice: "george".into(),
                quality: "fast".into(),
            },
            ch,
        ),
        "tts_start",
    )?;
    drain_tts(&rx, None)?;
    let (ch, rx) = channel::<SttEventDto>();
    ipc(
        c::transcribe_start(
            app.clone(),
            st_of(app),
            TranscribeRequest {
                source: SttSource::Path {
                    path: std::path::Path::new("fixtures/generated/synth/en_all.wav")
                        .canonicalize()
                        .map_err(|e| e.to_string())?
                        .display()
                        .to_string(),
                },
                language: "auto".into(),
            },
            ch,
        ),
        "transcribe_start",
    )?;
    loop {
        match rx
            .recv_timeout(Duration::from_secs(600))
            .map_err(|e| e.to_string())?
        {
            SttEventDto::Done { .. } => break,
            SttEventDto::Error { error } => return Err(error.message),
            _ => {}
        }
    }
    let wait: u64 = std::env::var("PARLO_IDLE_WAIT_S")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(70);
    phase("job-done", 3);
    let loaded = |tag: &str| {
        let st = st_of(app);
        println!(
            "LOADED {tag} stt={} tts={}",
            st.stt.is_loaded(),
            st.tts.is_loaded()
        );
    };
    loaded("job-done");
    phase("after-job", wait / 2);
    loaded("mid");
    std::thread::sleep(Duration::from_secs(wait - wait / 2));
    loaded("end");
    say("lifecycle complete");
    Ok(())
}

/// `PARLO_SELFTEST=pressure`: load both engines with a 15-minute idle timeout, announce `READY`, then wait (≤ 120 s) for
/// the macOS memory-pressure subscription to unload them. Driven by scripts/mac-memory-pressure-test.sh.
fn pressure_script(app: &AppHandle, say: &dyn Fn(&str)) -> R<()> {
    use parlo_core::models::IdleTimeout;
    let st = st_of(app);
    st.host
        .set_idle_timeout(IdleTimeout::After(Duration::from_secs(900)));
    {
        let _s = st.stt.acquire().map_err(|e| e.to_string())?;
        let _t = st.tts.acquire().map_err(|e| e.to_string())?;
    }
    ensure(st.stt.is_loaded() && st.tts.is_loaded(), "engines did not load")?;
    println!("READY engines loaded, idle timeout 15 min");
    use std::io::Write;
    let _ = std::io::stdout().flush();
    for i in 0..120 {
        std::thread::sleep(Duration::from_secs(1));
        if !st.stt.is_loaded() && !st.tts.is_loaded() {
            say(&format!("memory pressure unloaded both engines after {i} s"));
            return Ok(());
        }
    }
    Err("engines were not unloaded within 120 s of the memory-pressure event".into())
}

/// Experiment: one engine only, unload immediately, print RSS at each step (memory-return investigation).
fn lc_isolated(app: &AppHandle, only: &str) -> R<()> {
    use parlo_core::models::IdleTimeout;
    let rss = || {
        String::from_utf8_lossy(
            &std::process::Command::new("ps")
                .args(["-o", "rss=", "-p", &std::process::id().to_string()])
                .output()
                .map(|o| o.stdout)
                .unwrap_or_default(),
        )
        .trim()
        .parse::<u64>()
        .unwrap_or(0)
            / 1024
    };
    let st = st_of(app);
    println!("ISO baseline rss {} MB", rss());
    st.host.set_idle_timeout(IdleTimeout::Immediately);
    if only == "tts" {
        let (ch, rx) = channel::<TtsEventDto>();
        ipc(
            c::tts_start(
                app.clone(),
                st_of(app),
                TtsRequest {
                    source: TtsSource::Text {
                        text: "Hello from the isolated memory test. This is a short sentence."
                            .into(),
                    },
                    voice: "george".into(),
                    quality: "fast".into(),
                },
                ch,
            ),
            "tts_start",
        )?;
        drain_tts(&rx, None)?;
    } else {
        let (ch, rx) = channel::<SttEventDto>();
        ipc(
            c::transcribe_start(
                app.clone(),
                st_of(app),
                TranscribeRequest {
                    source: SttSource::Path {
                        path: std::path::Path::new("fixtures/generated/synth/en_all.wav")
                            .canonicalize()
                            .map_err(|e| e.to_string())?
                            .display()
                            .to_string(),
                    },
                    language: "auto".into(),
                },
                ch,
            ),
            "transcribe_start",
        )?;
        loop {
            match rx
                .recv_timeout(Duration::from_secs(600))
                .map_err(|e| e.to_string())?
            {
                SttEventDto::Done { .. } => break,
                SttEventDto::Error { error } => return Err(error.message),
                _ => {}
            }
        }
    }
    for i in 0..4 {
        std::thread::sleep(Duration::from_secs(2));
        println!(
            "ISO {only} +{}s: rss {} MB, stt_loaded={} tts_loaded={}",
            (i + 1) * 2,
            rss(),
            st.stt.is_loaded(),
            st.tts.is_loaded()
        );
    }
    Ok(())
}

fn st_of(app: &AppHandle) -> tauri::State<'_, Arc<AppState>> {
    app.state::<Arc<AppState>>()
}

pub fn spawn(app: AppHandle, state: Arc<AppState>) {
    let _ = std::thread::Builder::new()
        .name("selftest".into())
        .spawn(move || {
            let say = |m: &str| println!("  ✓ {m}");
            let mode = std::env::var("PARLO_SELFTEST").unwrap_or_default();
            match if mode == "mic" {
                mic_script(&app, &say)
            } else if mode == "pressure" {
                pressure_script(&app, &say)
            } else if mode == "lifecycle" {
                lifecycle_script(&app, &say)
            } else {
                script(&app, &state, &say)
            } {
                Ok(()) => {
                    println!("PARLO_SELFTEST_OK");
                    // same as a normal quit: engines must be released before exit() (see RunEvent::Exit)
                    state.host.unload_all_idle();
                    std::process::exit(0);
                }
                Err(e) => {
                    println!("PARLO_SELFTEST_FAIL: {e}");
                    state.host.unload_all_idle();
                    std::process::exit(1);
                }
            }
        });
}
