//! Tauri commands (BRIEF §5.4). Thin: parse → call parlo-core → map to DTOs. Long work runs on the job manager's
//! per-engine workers and reports through `Channel`s; nothing here blocks the UI thread for long.

use crate::ipc::*;
use crate::state::*;
use parlo_core::audio::encode;
use parlo_core::jobs::JobCtx;
use parlo_core::platform::DeviceSelector;
use parlo_core::recorder::sidecar::{RecState, Sidecar};
use parlo_core::recorder::{
    self, recovery, RecorderConfig, RecorderDeps, RecorderState, RecorderWarning,
};
use parlo_core::store::{Settings, Workspace, WorkspaceTranscript, WorkspaceTts};
use parlo_core::stt::pipeline::{
    input_key_for, run_transcription, LanguageSetting, Stage, SttEvent, SttJob,
};
use parlo_core::text::{self, Block};
use parlo_core::tts::{run_tts, Quality, TtsEvent, TtsJob};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_notification::NotificationExt;
use tauri_specta::Event;

type S<'a> = State<'a, Arc<AppState>>;

// =============================================================================================== app / settings
#[tauri::command]
#[specta::specta]
pub fn app_info(state: S<'_>) -> AppInfo {
    state.seen.lock().unwrap().insert("app_info");
    let m = &state.models;
    let mk = |name: &str, p: PathBuf| {
        let md = std::fs::metadata(&p).ok();
        ModelFileInfo {
            name: name.into(),
            present: md.is_some(),
            size_bytes: md.map(|m| m.len()).unwrap_or(0),
            path: p.display().to_string(),
        }
    };
    AppInfo {
        version: env!("CARGO_PKG_VERSION").into(),
        stt_engine: format!(
            "whisper.cpp · {}",
            DEFAULT_WHISPER_MODEL
                .trim_start_matches("ggml-")
                .trim_end_matches(".bin")
        ),
        tts_engine: "Kyutai PocketTTS (Candle)".into(),
        models: vec![
            mk("Speech recognition", m.whisper(DEFAULT_WHISPER_MODEL)),
            mk("Voice activity detection", m.vad()),
            mk("Speech synthesis", m.pockettts().join("hf")),
        ],
        data_dir: state.paths.data.display().to_string(),
        recordings_dir: state.paths.recordings().display().to_string(),
        logs_dir: state.paths.logs.display().to_string(),
        platform: std::env::consts::OS.into(),
    }
}

fn settings_dto(s: &Settings) -> SettingsDto {
    serde_json::from_value(serde_json::to_value(s).expect("settings serialize"))
        .expect("settings dto")
}

#[tauri::command]
#[specta::specta]
pub fn settings_get(state: S<'_>) -> SettingsDto {
    state.seen.lock().unwrap().insert("settings_get");
    settings_dto(&state.settings.lock().unwrap())
}

#[tauri::command]
#[specta::specta]
pub fn settings_update(state: S<'_>, patch: SettingsPatch) -> IpcResult<SettingsDto> {
    let mut patch =
        serde_json::to_value(&patch).map_err(|e| IpcError::new("invalid", e.to_string()))?;
    // "" means "follow the system default input" (the DTO avoids Option<Option<_>>)
    if let Some(o) = patch.as_object_mut() {
        o.retain(|_, v| !v.is_null()); // null = leave unchanged
        if o.get("input_device").and_then(|v| v.as_str()) == Some("") {
            o.insert("input_device".into(), serde_json::Value::Null);
        }
    }
    let mut cur = state.settings.lock().unwrap();
    let next = cur.apply_patch(&patch)?;
    next.save(&state.paths)?;
    state
        .host
        .set_idle_timeout(next.model_idle_timeout.to_timeout());
    *cur = next;
    Ok(settings_dto(&cur))
}

// =============================================================================================== microphone / devices
#[tauri::command]
#[specta::specta]
pub fn mic_permission_status(state: S<'_>) -> MicPermission {
    crate::native::mic_permission_status(&state)
}

#[tauri::command]
#[specta::specta]
pub async fn mic_permission_request(state: S<'_>) -> IpcResult<MicPermission> {
    Ok(crate::native::mic_permission_request(&state))
}

#[tauri::command]
#[specta::specta]
pub fn open_privacy_settings(kind: String) -> IpcResult<()> {
    crate::native::open_privacy_settings(&kind).map_err(|e| IpcError::new("other", e))
}

#[tauri::command]
#[specta::specta]
pub fn input_devices_list(state: S<'_>) -> IpcResult<Vec<InputDeviceDto>> {
    Ok(state
        .platform
        .input
        .devices()?
        .into_iter()
        .map(|d| InputDeviceDto {
            id: d.id,
            name: d.name,
            is_default: d.is_default,
        })
        .collect())
}

// =============================================================================================== recording
fn info_dto(sc: &Sidecar, wav: &Path, device: String) -> RecordingInfoDto {
    let rate = sc.sample_rate.max(1) as u64;
    RecordingInfoDto {
        id: sc.id.clone(),
        wav_path: wav.display().to_string(),
        sample_rate: sc.sample_rate,
        duration_ms: sc.frames_written * 1000 / rate,
        started_at: sc.started_at.clone(),
        state: format!("{:?}", sc.state).to_lowercase(),
        gaps: sc
            .gaps
            .iter()
            .map(|g| GapDto {
                at_ms: g.at_frame * 1000 / rate,
                reason: format!("{:?}", g.reason),
                gap_ms: g.gap_ms,
            })
            .collect(),
        device,
    }
}

pub fn emit_recorder_state(app: &AppHandle, state: &AppState) {
    let ev = match state.recorder.lock().unwrap().as_ref() {
        Some(h) => {
            let s = h.snapshot();
            RecorderStateEvent {
                state: match s.state {
                    RecorderState::Recording => "recording",
                    RecorderState::Paused => "paused",
                    RecorderState::Stopped => "idle",
                }
                .into(),
                id: Some(s.id),
                elapsed_ms: s.elapsed_ms,
                device: s.device,
                rms: s.level.rms,
                peak: s.level.peak,
                gaps: s.gaps as u32,
            }
        }
        None => RecorderStateEvent {
            state: "idle".into(),
            id: None,
            elapsed_ms: 0,
            device: String::new(),
            rms: 0.0,
            peak: 0.0,
            gaps: 0,
        },
    };
    let _ = ev.emit(app);
}

fn set_ticker_recording(state: &AppState, rec: bool) {
    let (m, cv) = &*state.ticker;
    m.lock().unwrap().recording = rec;
    cv.notify_all();
}

fn warning_dto(w: RecorderWarning) -> RecorderWarningDto {
    match w {
        RecorderWarning::DeviceChanged { from, to } => {
            RecorderWarningDto::DeviceChanged { from, to }
        }
        RecorderWarning::PinnedDeviceLost { pinned, fallback } => {
            RecorderWarningDto::PinnedDeviceLost { pinned, fallback }
        }
        RecorderWarning::StreamRestarted { reason } => {
            RecorderWarningDto::StreamRestarted { reason }
        }
        RecorderWarning::ResumedAfterSleep { gap_ms } => {
            RecorderWarningDto::ResumedAfterSleep { gap_ms }
        }
        RecorderWarning::LowDisk { free_mb } => RecorderWarningDto::LowDisk { free_mb },
        RecorderWarning::SilentInput => RecorderWarningDto::SilentInput,
        RecorderWarning::Clipping => RecorderWarningDto::Clipping,
        RecorderWarning::Stopped { reason } => RecorderWarningDto::Stopped { reason },
    }
}

#[tauri::command]
#[specta::specta]
pub fn recording_start(
    app: AppHandle,
    state: S<'_>,
    device_id: Option<String>,
) -> IpcResult<RecordingInfoDto> {
    let mut slot = state.recorder.lock().unwrap();
    if slot
        .as_ref()
        .is_some_and(|h| h.snapshot().state != RecorderState::Stopped)
    {
        return Err(IpcError::new("busy", "A recording is already in progress."));
    }
    let mut cfg = RecorderConfig::new(state.recordings_dir());
    cfg.device = match device_id {
        Some(d) => DeviceSelector::Named(d),
        None => state.settings.lock().unwrap().device_selector(),
    };
    let deps = RecorderDeps {
        input: state.platform.input.clone(),
        power: state.platform.power.clone(),
        sleep_wake: state.platform.sleep_wake.clone(),
        disk: state.platform.disk.clone(),
    };
    let (tx, rx) = crossbeam_channel::unbounded::<RecorderWarning>();
    let h = recorder::start(cfg, deps, tx)?;
    let app2 = app.clone();
    std::thread::Builder::new()
        .name("parlo-rec-warnings".into())
        .spawn(move || {
            for w in rx {
                let _ = RecorderWarningEvent {
                    warning: warning_dto(w),
                }
                .emit(&app2);
            }
        })
        .ok();
    let snap = h.snapshot();
    let sc = Sidecar::read(&state.recordings_dir().join(format!("{}.json", h.id())))
        .map_err(IpcError::from)?;
    let dto = info_dto(&sc, &state.recordings_dir().join(&sc.file), snap.device);
    *slot = Some(h);
    drop(slot);
    set_ticker_recording(&state, true);
    emit_recorder_state(&app, &state);
    Ok(dto)
}

fn with_recorder<T>(
    state: &AppState,
    f: impl FnOnce(&recorder::RecorderHandle) -> parlo_core::Result<T>,
) -> IpcResult<T> {
    let slot = state.recorder.lock().unwrap();
    let h = slot
        .as_ref()
        .ok_or_else(|| IpcError::new("invalid", "There is no recording in progress."))?;
    Ok(f(h)?)
}

#[tauri::command]
#[specta::specta]
pub fn recording_pause(app: AppHandle, state: S<'_>) -> IpcResult<()> {
    with_recorder(&state, |h| h.pause())?;
    emit_recorder_state(&app, &state);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn recording_resume(app: AppHandle, state: S<'_>) -> IpcResult<()> {
    with_recorder(&state, |h| h.resume())?;
    emit_recorder_state(&app, &state);
    Ok(())
}

/// Stop, finalize and return the recording. Whisper starts loading in parallel with the finalize (BRIEF §10).
#[tauri::command]
#[specta::specta]
pub fn recording_stop(app: AppHandle, state: S<'_>) -> IpcResult<RecordingInfoDto> {
    let h = state
        .recorder
        .lock()
        .unwrap()
        .take()
        .ok_or_else(|| IpcError::new("invalid", "There is no recording in progress."))?;
    let device = h.snapshot().device;
    set_ticker_recording(&state, false);
    let stt = state.stt.clone();
    std::thread::spawn(move || {
        let _ = stt.acquire(); // loads the speech model; the guard drop arms the idle timer
    });
    let info = h.stop()?;
    emit_recorder_state(&app, &state);
    Ok(info_dto(&info.sidecar, &info.wav, device))
}

#[tauri::command]
#[specta::specta]
pub fn recording_discard(app: AppHandle, state: S<'_>, id: String) -> IpcResult<()> {
    let active = state.recorder.lock().unwrap().take();
    set_ticker_recording(&state, false);
    match active {
        Some(h) if h.id() == id => h.discard()?,
        Some(h) => {
            *state.recorder.lock().unwrap() = Some(h);
            return Err(IpcError::new(
                "invalid",
                "That is not the recording in progress.",
            ));
        }
        None => {
            // an already-stopped recording (e.g. recovered): delete files
            let dir = state.recordings_dir();
            let _ = std::fs::remove_file(dir.join(format!("{id}.wav")));
            let _ = std::fs::remove_file(dir.join(format!("{id}.json")));
        }
    }
    emit_recorder_state(&app, &state);
    Ok(())
}

// =============================================================================================== recovery
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum JobRequestFile {
    Stt {
        path: String,
        recording_id: Option<String>,
        source_name: String,
        language: String,
    },
    Tts {
        blocks: Vec<(String, bool)>,
        voice: String,
        quality: String,
        source_name: Option<String>,
    },
}

fn job_dir_for(state: &AppState, id: &str) -> PathBuf {
    state.paths.jobs().join(id)
}

fn read_request(dir: &Path) -> Option<JobRequestFile> {
    serde_json::from_slice(&std::fs::read(dir.join("request.json")).ok()?).ok()
}

fn write_request(dir: &Path, r: &JobRequestFile) {
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(
        dir.join("request.json"),
        serde_json::to_vec(r).unwrap_or_default(),
    );
}

#[tauri::command]
#[specta::specta]
pub fn recoverables_list(state: S<'_>) -> IpcResult<Vec<RecoverableDto>> {
    let mut out = Vec::new();
    for r in recovery::recover_all(&state.recordings_dir())? {
        out.push(RecoverableDto::Recording {
            id: r.id.clone(),
            wav_path: r.wav.display().to_string(),
            duration_ms: r.sidecar.duration_ms,
            started_at: r.sidecar.started_at.clone(),
            saved_on_quit: r.sidecar.stop_reason.as_deref() == Some(recovery::QUIT_REASON),
        });
    }
    if let Ok(rd) = std::fs::read_dir(state.paths.jobs()) {
        for e in rd.flatten() {
            let id = e.file_name().to_string_lossy().to_string();
            match read_request(&e.path()) {
                Some(JobRequestFile::Stt { source_name, .. }) => out.push(RecoverableDto::SttJob {
                    job_id: id,
                    source_name,
                }),
                Some(JobRequestFile::Tts { blocks, voice, .. }) => {
                    let preview: String = blocks
                        .iter()
                        .map(|b| b.0.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                        .chars()
                        .take(80)
                        .collect();
                    out.push(RecoverableDto::TtsJob {
                        job_id: id,
                        text_preview: preview,
                        voice,
                    })
                }
                None => {}
            }
        }
    }
    Ok(out)
}

#[tauri::command]
#[specta::specta]
pub fn recoverable_discard(state: S<'_>, kind: String, id: String) -> IpcResult<()> {
    if id.contains('/') || id.contains("..") {
        return Err(IpcError::new("invalid", "Invalid id."));
    }
    match kind.as_str() {
        "recording" => {
            let dir = state.recordings_dir();
            let _ = std::fs::remove_file(dir.join(format!("{id}.wav")));
            let _ = std::fs::remove_file(dir.join(format!("{id}.json")));
        }
        _ => {
            let _ = std::fs::remove_dir_all(job_dir_for(&state, &id));
        }
    }
    Ok(())
}

// =============================================================================================== transcription
fn stage_name(s: Stage) -> &'static str {
    match s {
        Stage::Preparing => "preparing",
        Stage::DetectingSpeech => "detecting_speech",
        Stage::DetectingLanguage => "detecting_language",
        Stage::Transcribing => "transcribing",
    }
}

fn notify_if_no_window(app: &AppHandle, title: &str, body: &str) {
    if app.webview_windows().is_empty() {
        let _ = app.notification().builder().title(title).body(body).show();
    }
}

#[allow(clippy::too_many_arguments)] // one internal entry point shared by transcribe_start and stt_resume
fn start_stt(
    app: AppHandle,
    state: Arc<AppState>,
    job_id: String,
    path: PathBuf,
    recording_id: Option<String>,
    source_name: String,
    language: String,
    on_event: Channel<SttEventDto>,
) -> IpcResult<u32> {
    let dir = job_dir_for(&state, &job_id);
    write_request(
        &dir,
        &JobRequestFile::Stt {
            path: path.display().to_string(),
            recording_id: recording_id.clone(),
            source_name: source_name.clone(),
            language: language.clone(),
        },
    );
    let mut job = SttJob::new(&path, dir.join("work"), state.paths.decode_cache());
    job.language = if language == "auto" {
        LanguageSetting::Auto
    } else {
        LanguageSetting::Code(language.clone())
    };
    job.threads = crate::native::stt_threads(&state.settings.lock().unwrap());
    job.fallback_decoder = state.platform.fallback_decoder.clone();
    let stt = state.stt.clone();
    let st2 = state.clone();
    let label = format!("Transcribe {source_name}");
    let id = state.jobs.submit(
        parlo_core::models::EngineKind::Stt,
        &label,
        move |ctx: &JobCtx| {
            let _ = on_event.send(SttEventDto::Stage {
                stage: "loading_model".into(),
            });
            let mut guard = match stt.acquire() {
                Ok(g) => g,
                Err(e) => {
                    let _ = on_event.send(SttEventDto::Error { error: e.into() });
                    return Err(parlo_core::Error::Other("model load failed".into()));
                }
            };
            let SttBundle { engine, vad } = &mut *guard;
            let mut idx = 0u32;
            let res = run_transcription(
                &job,
                engine,
                vad,
                &mut |e: SttEvent| {
                    let dto = match e {
                        SttEvent::Stage(s) => SttEventDto::Stage {
                            stage: stage_name(s).into(),
                        },
                        SttEvent::StageProgress(f) => SttEventDto::StageProgress { fraction: f },
                        SttEvent::Language(l) => SttEventDto::Language {
                            code: l.code,
                            prob: l.prob,
                        },
                        SttEvent::Progress {
                            done_s,
                            total_s,
                            eta_ms,
                        } => SttEventDto::Progress {
                            done_s,
                            total_s,
                            eta_ms,
                        },
                        SttEvent::Paragraph(p) => {
                            idx += 1;
                            SttEventDto::Paragraph {
                                index: p.index as u32,
                                text: p.text,
                                start_s: p.start_s,
                                end_s: p.end_s,
                            }
                        }
                    };
                    let _ = on_event.send(dto);
                },
                &ctx.cancel,
            );
            drop(guard);
            match res {
                Ok(t) => {
                    let dto = TranscriptDto {
                        text: t.text.clone(),
                        language: t.language.as_ref().map(|l| l.code.clone()),
                        language_prob: t.language.as_ref().map(|l| l.prob),
                        duration_s: t.duration_s,
                        speech_s: t.speech_s,
                        words: t.word_count as u32,
                        processing_s: t.processing_s,
                        source_name: source_name.clone(),
                        recording_id: recording_id.clone(),
                    };
                    {
                        let mut ws = st2.workspace.lock().unwrap();
                        ws.transcript = Some(WorkspaceTranscript {
                            text: t.text.clone(),
                            source_name: source_name.clone(),
                            language: dto.language.clone(),
                            duration_s: t.duration_s,
                            words: t.word_count,
                            edited: false,
                            exported_or_copied: false,
                            recording_id: recording_id.clone(),
                        });
                        let _ = ws.save(&st2.paths);
                    }
                    if let Some(rid) = &recording_id {
                        let p = st2.recordings_dir().join(format!("{rid}.json"));
                        if let Ok(mut sc) = Sidecar::read(&p) {
                            sc.transcribed = true;
                            let _ = sc.write(&st2.recordings_dir());
                        }
                    }
                    let _ = std::fs::remove_dir_all(&dir);
                    let _ = on_event.send(SttEventDto::Done { transcript: dto });
                    notify_if_no_window(&app, "Transcript ready", &source_name);
                    Ok(())
                }
                Err(parlo_core::Error::Cancelled) => {
                    if ctx.cancel.load(Ordering::SeqCst) {
                        let _ = std::fs::remove_dir_all(&dir); // an explicit cancel discards the checkpoint
                                                               // the UI waits for a terminal event on this channel to leave its "Cancelling…" state
                        let _ = on_event.send(SttEventDto::Error {
                            error: IpcError::new("cancelled", "Cancelled"),
                        });
                    }
                    Err(parlo_core::Error::Cancelled)
                }
                Err(e) => {
                    let _ = on_event.send(SttEventDto::Error {
                        error: IpcError::from(parlo_core::Error::Other(e.to_string())),
                    });
                    Err(e)
                }
            }
        },
    );
    Ok(id as u32)
}

#[tauri::command]
#[specta::specta]
pub fn transcribe_start(
    app: AppHandle,
    state: S<'_>,
    request: TranscribeRequest,
    on_event: Channel<SttEventDto>,
) -> IpcResult<u32> {
    let (path, recording_id, name) = match request.source {
        SttSource::Path { path } => {
            let p = PathBuf::from(&path);
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.clone());
            // let the mini player read the source through the asset protocol
            let _ = app.asset_protocol_scope().allow_file(&p);
            (p, None, name)
        }
        SttSource::Recording { recording_id } => {
            if recording_id.contains('/') || recording_id.contains("..") {
                return Err(IpcError::new("invalid", "Invalid recording id."));
            }
            (
                state.recordings_dir().join(format!("{recording_id}.wav")),
                Some(recording_id.clone()),
                format!("Recording {}", recording_id.trim_start_matches("rec-")),
            )
        }
    };
    if !path.exists() {
        return Err(IpcError::new(
            "io",
            format!("The file {} no longer exists.", path.display()),
        ));
    }
    let job_id = format!("stt-{:016x}", fnv(input_key_for(&path).as_bytes()));
    start_stt(
        app,
        state.inner().clone(),
        job_id,
        path,
        recording_id,
        name,
        request.language,
        on_event,
    )
}

#[tauri::command]
#[specta::specta]
pub fn stt_resume(
    app: AppHandle,
    state: S<'_>,
    job_id: String,
    on_event: Channel<SttEventDto>,
) -> IpcResult<u32> {
    if job_id.contains('/') || job_id.contains("..") {
        return Err(IpcError::new("invalid", "Invalid job id."));
    }
    match read_request(&job_dir_for(&state, &job_id)) {
        Some(JobRequestFile::Stt {
            path,
            recording_id,
            source_name,
            language,
        }) => start_stt(
            app,
            state.inner().clone(),
            job_id,
            PathBuf::from(path),
            recording_id,
            source_name,
            language,
            on_event,
        ),
        _ => Err(IpcError::new("invalid", "That job can't be resumed.")),
    }
}

fn fnv(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf29ce484222325u64, |h, x| {
        (h ^ *x as u64).wrapping_mul(0x100000001b3)
    })
}

#[tauri::command]
#[specta::specta]
pub fn job_cancel(state: S<'_>, job_id: u32) {
    state.jobs.cancel(job_id as u64);
}

// =============================================================================================== text → speech
fn quality_of(s: &str) -> Quality {
    if s == "fast" {
        Quality::Fast
    } else {
        Quality::Standard
    }
}

#[allow(clippy::too_many_arguments)]
fn start_tts(
    app: AppHandle,
    state: Arc<AppState>,
    job_id: String,
    blocks: Vec<Block>,
    voice: String,
    quality: String,
    source_name: Option<String>,
    on_event: Channel<TtsEventDto>,
) -> IpcResult<u32> {
    let dir = job_dir_for(&state, &job_id);
    write_request(
        &dir,
        &JobRequestFile::Tts {
            blocks: blocks.iter().map(|b| (b.text.clone(), b.heading)).collect(),
            voice: voice.clone(),
            quality: quality.clone(),
            source_name: source_name.clone(),
        },
    );
    let tts = state.tts.clone();
    let st2 = state.clone();
    let outputs = state.paths.tts_outputs();
    let _ = app.asset_protocol_scope().allow_directory(&outputs, true);
    let job = TtsJob {
        blocks: blocks.clone(),
        voice: voice.clone(),
        quality: quality_of(&quality),
        outputs_dir: outputs,
        job_dir: dir.join("work"),
    };
    let label = "Generate speech".to_string();
    let id = state.jobs.submit(
        parlo_core::models::EngineKind::Tts,
        &label,
        move |ctx: &JobCtx| {
            let _ = on_event.send(TtsEventDto::Stage {
                stage: "loading_model".into(),
            });
            let mut guard = match tts.acquire() {
                Ok(g) => g,
                Err(e) => {
                    let _ = on_event.send(TtsEventDto::Error { error: e.into() });
                    return Err(parlo_core::Error::Other("model load failed".into()));
                }
            };
            let _ = on_event.send(TtsEventDto::Stage {
                stage: "generating".into(),
            });
            let res = run_tts(
                &job,
                &mut **guard,
                &mut |e: TtsEvent| {
                    let dto = match e {
                        TtsEvent::Progress {
                            done_chars,
                            total_chars,
                            eta_ms,
                        } => TtsEventDto::Progress {
                            done_chars: done_chars as u32,
                            total_chars: total_chars as u32,
                            eta_ms,
                        },
                        TtsEvent::SegmentReady {
                            index,
                            path,
                            duration_ms,
                        } => TtsEventDto::SegmentReady {
                            index: index as u32,
                            path: path.display().to_string(),
                            duration_ms,
                        },
                        TtsEvent::Done { .. } => return, // sent below with the full segment list
                    };
                    let _ = on_event.send(dto);
                },
                &ctx.cancel,
            );
            drop(guard);
            match res {
                Ok(out) => {
                    {
                        let mut ws = st2.workspace.lock().unwrap();
                        ws.tts = Some(WorkspaceTts {
                            text: blocks
                                .iter()
                                .map(|b| b.text.as_str())
                                .collect::<Vec<_>>()
                                .join("\n\n"),
                            voice: voice.clone(),
                            output_id: Some(out.output_id.clone()),
                            source_name: source_name.clone(),
                        });
                        let _ = ws.save(&st2.paths);
                    }
                    let _ = std::fs::remove_dir_all(&dir);
                    let _ = on_event.send(TtsEventDto::Done {
                        output_id: out.output_id.clone(),
                        duration_ms: out.meta.duration_ms,
                        segments: out
                            .segments
                            .iter()
                            .map(|p| p.display().to_string())
                            .collect(),
                        cache_hit: out.cache_hit,
                    });
                    notify_if_no_window(
                        &app,
                        "Speech ready",
                        &format!("{:.0} s of audio", out.meta.duration_ms as f64 / 1000.0),
                    );
                    Ok(())
                }
                Err(parlo_core::Error::Cancelled) => {
                    if ctx.cancel.load(Ordering::SeqCst) {
                        let _ = std::fs::remove_dir_all(&dir);
                        let _ = on_event.send(TtsEventDto::Error {
                            error: IpcError::new("cancelled", "Cancelled"),
                        });
                    }
                    Err(parlo_core::Error::Cancelled)
                }
                Err(e) => {
                    let _ = on_event.send(TtsEventDto::Error {
                        error: IpcError::from(parlo_core::Error::Other(e.to_string())),
                    });
                    Err(e)
                }
            }
        },
    );
    Ok(id as u32)
}

#[tauri::command]
#[specta::specta]
pub fn tts_start(
    app: AppHandle,
    state: S<'_>,
    request: TtsRequest,
    on_event: Channel<TtsEventDto>,
) -> IpcResult<u32> {
    let (blocks, source_name) = match request.source {
        TtsSource::Text { text } => (text::text_to_blocks(&text), None),
        TtsSource::Import { import_id } => {
            let imports = state.imports.lock().unwrap();
            let d = imports.get(&import_id).ok_or_else(|| {
                IpcError::new(
                    "invalid",
                    "That imported document is no longer available. Open it again.",
                )
            })?;
            (d.blocks.clone(), Some(d.file_name.clone()))
        }
    };
    if blocks.iter().all(|b| b.text.trim().is_empty()) {
        return Err(IpcError::new("invalid", "There is no text to read."));
    }
    let key: String = blocks
        .iter()
        .map(|b| b.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let job_id = format!(
        "tts-{:016x}",
        fnv(format!("{}|{}|{}", request.voice, request.quality, key).as_bytes())
    );
    start_tts(
        app,
        state.inner().clone(),
        job_id,
        blocks,
        request.voice,
        request.quality,
        source_name,
        on_event,
    )
}

#[tauri::command]
#[specta::specta]
pub fn tts_resume(
    app: AppHandle,
    state: S<'_>,
    job_id: String,
    on_event: Channel<TtsEventDto>,
) -> IpcResult<u32> {
    if job_id.contains('/') || job_id.contains("..") {
        return Err(IpcError::new("invalid", "Invalid job id."));
    }
    match read_request(&job_dir_for(&state, &job_id)) {
        Some(JobRequestFile::Tts {
            blocks,
            voice,
            quality,
            source_name,
        }) => {
            let blocks = blocks
                .into_iter()
                .map(|(t, h)| Block {
                    text: t,
                    heading: h,
                })
                .collect();
            start_tts(
                app,
                state.inner().clone(),
                job_id,
                blocks,
                voice,
                quality,
                source_name,
                on_event,
            )
        }
        _ => Err(IpcError::new("invalid", "That job can't be resumed.")),
    }
}

#[tauri::command]
#[specta::specta]
pub fn voices_list(state: S<'_>) -> Vec<VoiceDto> {
    state.seen.lock().unwrap().insert("voices_list");
    let previews = state.models.previews_dir();
    parlo_core::tts::pocket::discover_voices(&state.models)
        .into_iter()
        .map(|v| {
            let p = previews.join(format!("{}.mp3", v.id));
            VoiceDto {
                preview_path: p.exists().then(|| p.display().to_string()),
                id: v.id,
                label: v.label,
            }
        })
        .collect()
}

// =============================================================================================== documents
#[tauri::command]
#[specta::specta]
pub fn text_import(state: S<'_>, path: String) -> IpcResult<ImportResultDto> {
    let p = PathBuf::from(&path);
    let imp = text::import_file(&p, state.platform.pdf.as_deref())?;
    let file_name = p
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or(path);
    let full = imp.text();
    let large = imp.stats.chars > text::import::LARGE_TEXT_CHARS;
    let preview: String = full.chars().take(5000).collect();
    let id = state.new_import_id();
    let dto = ImportResultDto {
        import_id: id.clone(),
        file_name: file_name.clone(),
        kind: imp.kind.into(),
        text: if large { None } else { Some(full) },
        preview,
        large,
        chars: imp.stats.chars as u32,
        words: imp.stats.words as u32,
        pages: imp.stats.pages.map(|p| p as u32),
        chapters: imp.stats.chapters.map(|c| c as u32),
        est_minutes: imp.stats.est_minutes,
        warnings: imp.warnings,
    };
    let mut imports = state.imports.lock().unwrap();
    if imports.len() > 4 {
        imports.clear(); // keep memory bounded: only the latest few imports are retained
    }
    imports.insert(
        id,
        ImportedDoc {
            blocks: imp.blocks,
            file_name,
        },
    );
    Ok(dto)
}

#[tauri::command]
#[specta::specta]
pub fn text_import_full(state: S<'_>, import_id: String) -> IpcResult<String> {
    let imports = state.imports.lock().unwrap();
    let d = imports.get(&import_id).ok_or_else(|| {
        IpcError::new("invalid", "That imported document is no longer available.")
    })?;
    Ok(text::blocks_to_text(&d.blocks))
}

// =============================================================================================== export
fn safe_dest(dest: &str) -> IpcResult<PathBuf> {
    let p = PathBuf::from(dest);
    if !p.is_absolute() {
        return Err(IpcError::new(
            "invalid",
            "The destination must be an absolute path.",
        ));
    }
    Ok(p)
}

fn output_segments(state: &AppState, output_id: &str) -> IpcResult<Vec<PathBuf>> {
    if output_id.contains('/') || output_id.contains("..") {
        return Err(IpcError::new("invalid", "Invalid output id."));
    }
    let dir = state.paths.tts_outputs().join(output_id);
    let meta: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("output.json")).map_err(|_| {
            IpcError::new(
                "io",
                "That audio is no longer available. Generate it again.",
            )
        })?)
        .map_err(|e| IpcError::new("io", e.to_string()))?;
    Ok(meta["segments"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| s["file"].as_str().map(|f| dir.join(f)))
                .collect()
        })
        .unwrap_or_default())
}

#[tauri::command]
#[specta::specta]
pub async fn export_tts(
    state: S<'_>,
    output_id: String,
    format: AudioExportFormat,
    dest: String,
) -> IpcResult<String> {
    let dest = safe_dest(&dest)?;
    let segs = output_segments(&state, &output_id)?;
    let aac = state.platform.aac.clone();
    let tmp = state.paths.cache.clone();
    let d2 = dest.clone();
    tauri::async_runtime::spawn_blocking(move || -> parlo_core::Result<()> {
        match format {
            AudioExportFormat::Wav => encode::export_wav(&segs, &d2).map(|_| ()),
            AudioExportFormat::Mp3 => encode::export_mp3(&segs, &d2, 96),
            AudioExportFormat::M4a => encode::export_m4a(&*aac, &segs, &d2, 96, &tmp),
        }
    })
    .await
    .map_err(|e| IpcError::new("other", e.to_string()))??;
    Ok(dest.display().to_string())
}

#[tauri::command]
#[specta::specta]
pub fn export_transcript(
    state: S<'_>,
    text: String,
    format: TranscriptExportFormat,
    dest: String,
    meta: TranscriptMeta,
) -> IpcResult<String> {
    let dest = safe_dest(&dest)?;
    let body = match format {
        TranscriptExportFormat::Txt => text,
        TranscriptExportFormat::Md => format!(
            "# Transcript: {}\n\n- **Source:** {}\n- **Date:** {}\n- **Duration:** {}\n- **Language:** {}\n\n---\n\n{}\n",
            meta.source_name,
            meta.source_name,
            chrono::Local::now().format("%Y-%m-%d %H:%M"),
            fmt_duration(meta.duration_s),
            meta.language.as_deref().unwrap_or("unknown"),
            text
        ),
    };
    std::fs::write(&dest, body)
        .map_err(|e| IpcError::new("io", format!("Couldn't save the file: {e}")))?;
    if let Some(t) = state.workspace.lock().unwrap().transcript.as_mut() {
        t.exported_or_copied = true;
    }
    Ok(dest.display().to_string())
}

fn fmt_duration(s: f64) -> String {
    let t = s.round() as u64;
    format!("{}:{:02}:{:02}", t / 3600, (t / 60) % 60, t % 60)
}

#[tauri::command]
#[specta::specta]
pub async fn export_recording(
    state: S<'_>,
    recording_id: String,
    format: AudioExportFormat,
    dest: String,
) -> IpcResult<String> {
    if recording_id.contains('/') || recording_id.contains("..") {
        return Err(IpcError::new("invalid", "Invalid recording id."));
    }
    let dest = safe_dest(&dest)?;
    let wav = state.recordings_dir().join(format!("{recording_id}.wav"));
    let aac = state.platform.aac.clone();
    let tmp = state.paths.cache.clone();
    let d2 = dest.clone();
    tauri::async_runtime::spawn_blocking(move || -> parlo_core::Result<()> {
        match format {
            AudioExportFormat::Wav => std::fs::copy(&wav, &d2)
                .map(|_| ())
                .map_err(|e| parlo_core::Error::io(&d2, e)),
            AudioExportFormat::M4a => encode::export_m4a(&*aac, &[wav], &d2, 96, &tmp),
            AudioExportFormat::Mp3 => encode::export_mp3(&[wav], &d2, 96),
        }
    })
    .await
    .map_err(|e| IpcError::new("other", e.to_string()))??;
    Ok(dest.display().to_string())
}

// =============================================================================================== workspace / misc
fn ws_dto(state: &AppState, ws: &Workspace) -> WorkspaceDto {
    WorkspaceDto {
        transcript: ws.transcript.as_ref().map(|t| WorkspaceTranscriptDto {
            text: t.text.clone(),
            source_name: t.source_name.clone(),
            language: t.language.clone(),
            duration_s: t.duration_s,
            words: t.words as u32,
            edited: t.edited,
            exported_or_copied: t.exported_or_copied,
            recording_id: t.recording_id.clone(),
        }),
        tts: ws.tts.as_ref().map(|t| {
            let (segments, dur) = match &t.output_id {
                Some(id) => {
                    let segs = output_segments(state, id).unwrap_or_default();
                    let dur = std::fs::read(state.paths.tts_outputs().join(id).join("output.json"))
                        .ok()
                        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                        .and_then(|v| v["duration_ms"].as_u64());
                    (
                        segs.iter()
                            .filter(|p| p.exists())
                            .map(|p| p.display().to_string())
                            .collect(),
                        dur,
                    )
                }
                None => (vec![], None),
            };
            WorkspaceTtsDto {
                text: t.text.clone(),
                voice: t.voice.clone(),
                output_id: t.output_id.clone(),
                source_name: t.source_name.clone(),
                segments,
                duration_ms: dur,
            }
        }),
    }
}

#[tauri::command]
#[specta::specta]
pub fn workspace_get(app: AppHandle, state: S<'_>) -> WorkspaceDto {
    state.seen.lock().unwrap().insert("workspace_get");
    let ws = state.workspace.lock().unwrap().clone();
    let _ = app
        .asset_protocol_scope()
        .allow_directory(state.paths.tts_outputs(), true);
    let _ = app
        .asset_protocol_scope()
        .allow_directory(state.recordings_dir(), true);
    let _ = app
        .asset_protocol_scope()
        .allow_directory(state.models.previews_dir(), true);
    ws_dto(&state, &ws)
}

#[tauri::command]
#[specta::specta]
pub fn workspace_update(state: S<'_>, patch: WorkspacePatch) -> IpcResult<()> {
    let mut ws = state.workspace.lock().unwrap();
    if patch.clear_transcript {
        ws.transcript = None;
    }
    if let Some(t) = patch.transcript {
        ws.transcript = Some(WorkspaceTranscript {
            text: t.text,
            source_name: t.source_name,
            language: t.language,
            duration_s: t.duration_s,
            words: t.words as usize,
            edited: t.edited,
            exported_or_copied: t.exported_or_copied,
            recording_id: t.recording_id,
        });
    }
    if patch.clear_tts {
        ws.tts = None;
    }
    if let Some(t) = patch.tts {
        ws.tts = Some(WorkspaceTts {
            text: t.text,
            voice: t.voice,
            output_id: t.output_id,
            source_name: t.source_name,
        });
    }
    ws.save(&state.paths)?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn reveal_in_finder(path: String) -> IpcResult<()> {
    tauri_plugin_opener::reveal_item_in_dir(PathBuf::from(path))
        .map_err(|e| IpcError::new("io", e.to_string()))
}

/// The UI tells the core whether it is visible, so the level meter only runs while recording *and* visible.
#[tauri::command]
#[specta::specta]
pub fn ui_visibility(app: AppHandle, state: S<'_>, visible: bool) {
    state.seen.lock().unwrap().insert("ui_visibility");
    {
        let (m, cv) = &*state.ticker;
        m.lock().unwrap().visible = visible;
        cv.notify_all();
    }
    // A (re)opened window reattaches to a recording the core is already making — including a paused one, which has no
    // level ticks to carry the state.
    if visible {
        emit_recorder_state(&app, &state);
    }
}

#[tauri::command]
#[specta::specta]
pub fn diagnostics_text(state: S<'_>) -> String {
    format!(
        "Parlo {} ({} {})\nSTT: {}\nTTS: pockettts-candle\nModels dir: {}\nData dir: {}\n\n--- last 200 log lines ---\n{}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        DEFAULT_WHISPER_MODEL,
        state.models.root.display(),
        state.paths.data.display(),
        parlo_core::logging::tail_lines(&state.paths.logs, 200)
    )
}

/// Final answer to the quit confirmation (stop and save the recording, then exit).
#[tauri::command]
#[specta::specta]
pub fn quit_confirmed(app: AppHandle, state: S<'_>) -> IpcResult<()> {
    stop_recording_for_quit(&state);
    state.quitting.store(true, Ordering::SeqCst);
    app.exit(0);
    Ok(())
}

/// ⌘Q → "Stop and save": finish the recording and tag it so the next launch offers to transcribe it.
pub fn stop_recording_for_quit(state: &AppState) {
    if let Some(h) = state.recorder.lock().unwrap().take() {
        let id = h.id().to_string();
        let _ = h.stop();
        let _ = recovery::mark_saved_on_quit(&state.recordings_dir(), &id);
    }
}

pub fn _assert_send_sync() {
    fn is<T: Send + Sync>() {}
    is::<AppState>();
    let _ = RecState::Finalized;
}

// =============================================================================================== storage
fn dir_size(p: &std::path::Path) -> u64 {
    let Ok(md) = std::fs::symlink_metadata(p) else {
        return 0;
    };
    if md.is_file() {
        return md.len();
    }
    if !md.is_dir() {
        return 0;
    }
    std::fs::read_dir(p)
        .map(|rd| rd.flatten().map(|e| dir_size(&e.path())).sum())
        .unwrap_or(0)
}

fn storage_entries(state: &AppState) -> Vec<StorageEntryDto> {
    let m = &state.models;
    let entry = |kind: &str, label: &str, bytes: u64, path: PathBuf| StorageEntryDto {
        kind: kind.into(),
        label: label.into(),
        bytes,
        path: path.display().to_string(),
    };
    let models =
        dir_size(&m.whisper(DEFAULT_WHISPER_MODEL)) + dir_size(&m.vad()) + dir_size(&m.pockettts());
    vec![
        entry(
            "recordings",
            "Recordings",
            dir_size(&state.paths.recordings()),
            state.paths.recordings(),
        ),
        entry(
            "outputs",
            "Generated speech",
            dir_size(&state.paths.tts_outputs()),
            state.paths.tts_outputs(),
        ),
        entry(
            "cache",
            "Cache",
            dir_size(&state.paths.decode_cache()),
            state.paths.decode_cache(),
        ),
        entry("models", "Models", models, m.pockettts()),
    ]
}

#[tauri::command]
#[specta::specta]
pub fn storage_usage(state: S<'_>) -> IpcResult<Vec<StorageEntryDto>> {
    Ok(storage_entries(&state))
}

/// Explicit user action: delete generated speech (`outputs`) or the decode cache (`cache`). Recordings are never touched here.
#[tauri::command]
#[specta::specta]
pub fn storage_clear(state: S<'_>, kind: String) -> IpcResult<Vec<StorageEntryDto>> {
    let dir = match kind.as_str() {
        "outputs" => state.paths.tts_outputs(),
        "cache" => state.paths.decode_cache(),
        other => return Err(IpcError::new("invalid", format!("can't clear '{other}'"))),
    };
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let p = e.path();
            let _ = if p.is_dir() {
                std::fs::remove_dir_all(&p)
            } else {
                std::fs::remove_file(&p)
            };
        }
    }
    if kind == "outputs" {
        // the workspace's current output no longer exists
        let mut ws = state.workspace.lock().unwrap();
        if let Some(t) = ws.tts.as_mut() {
            t.output_id = None;
        }
        ws.save(&state.paths)?;
    }
    Ok(storage_entries(&state))
}
