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
        // no PDF extractor on Linux: must be a clear error, not a crash
        ensure(
            r.is_err(),
            "PDF import without PDFKit should report a clear error on Linux",
        )?;
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
    }
    say("tts: WAV + MP3 export (M4A unavailable on Linux, reported cleanly)");
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

pub fn spawn(app: AppHandle, state: Arc<AppState>) {
    let _ = std::thread::Builder::new()
        .name("selftest".into())
        .spawn(move || {
            let say = |m: &str| println!("  ✓ {m}");
            match script(&app, &state, &say) {
                Ok(()) => {
                    println!("PARLO_SELFTEST_OK");
                    std::process::exit(0);
                }
                Err(e) => {
                    println!("PARLO_SELFTEST_FAIL: {e}");
                    std::process::exit(1);
                }
            }
        });
}
