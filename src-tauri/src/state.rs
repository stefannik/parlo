//! Shared application state: stores, engine slots (lazy, idle-unloaded), job manager, recorder, imports.

use crate::ipc::*;
use parlo_core::jobs::{JobInfo, JobManager, JobState};
use parlo_core::models::{EngineKind, ModelHost, ModelState, ModelsEvent, Slot};
use parlo_core::paths::{AppPaths, ModelPaths};
use parlo_core::platform::Platform;
use parlo_core::recorder::RecorderHandle;
use parlo_core::store::{Settings, Workspace};
use parlo_core::stt::whisper::{SileroVad, WhisperConfig, WhisperEngine};
use parlo_core::text::Block;
use parlo_core::tts::TtsEngine;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Condvar, Mutex};
use tauri::AppHandle;
use tauri_specta::Event;

/// Default speech-recognition model (see docs/DECISIONS.md "STT model selection").
pub const DEFAULT_WHISPER_MODEL: &str = "ggml-large-v3-turbo-q8_0.bin";

/// The speech model actually loaded: the default, or `PARLO_WHISPER_MODEL` (used by the self-test to run a faster model).
pub fn whisper_model_name() -> String {
    std::env::var("PARLO_WHISPER_MODEL").unwrap_or_else(|_| DEFAULT_WHISPER_MODEL.to_string())
}

pub struct SttBundle {
    pub engine: WhisperEngine,
    pub vad: SileroVad,
}

pub struct ImportedDoc {
    pub blocks: Vec<Block>,
    pub file_name: String,
}

pub struct AppState {
    pub paths: AppPaths,
    pub models: ModelPaths,
    pub settings: Mutex<Settings>,
    pub workspace: Mutex<Workspace>,
    pub platform: Platform,
    pub host: Arc<ModelHost>,
    pub stt: Arc<Slot<SttBundle>>,
    pub tts: Arc<Slot<Box<dyn TtsEngine>>>,
    pub jobs: JobManager,
    pub recorder: Mutex<Option<RecorderHandle>>,
    pub imports: Mutex<HashMap<String, ImportedDoc>>,
    pub next_import: Mutex<u64>,
    /// (recording && ui visible) gate for the level-meter ticker: the ticker thread sleeps on this condvar otherwise
    pub ticker: Arc<(Mutex<TickerGate>, Condvar)>,
    pub quitting: AtomicBool,
    /// commands the UI has called at least once (read by the `PARLO_SMOKE` self-test)
    pub seen: Mutex<std::collections::BTreeSet<&'static str>>,
}

#[derive(Default)]
pub struct TickerGate {
    pub recording: bool,
    pub visible: bool,
}

pub fn engine_str(k: EngineKind) -> &'static str {
    match k {
        EngineKind::Stt => "stt",
        EngineKind::Tts => "tts",
    }
}

impl AppState {
    pub fn new(app: &AppHandle, paths: AppPaths, models: ModelPaths) -> Arc<Self> {
        let settings = Settings::load(&paths);
        let workspace = Workspace::load(&paths);
        let platform = crate::native::platform();

        let app_for_models = app.clone();
        let host = ModelHost::new(
            settings.model_idle_timeout.to_timeout(),
            Arc::new(move |e: ModelsEvent| {
                let _ = ModelsStateEvent {
                    engine: engine_str(e.engine).into(),
                    state: match e.state {
                        ModelState::Unloaded => "unloaded",
                        ModelState::Loading => "loading",
                        ModelState::Ready => "ready",
                    }
                    .into(),
                    unload_in_ms: e.unload_in_ms,
                }
                .emit(&app_for_models);
            }),
        );

        let m = models.clone();
        let stt = host.slot(EngineKind::Stt, move || {
            let engine =
                WhisperEngine::load(&m.whisper(&whisper_model_name()), WhisperConfig::default())?;
            let vad = SileroVad::load(&m.vad(), 300)?;
            Ok(SttBundle { engine, vad })
        });
        let m = models.clone();
        let tts: Arc<Slot<Box<dyn TtsEngine>>> = host.slot(EngineKind::Tts, move || {
            let e = parlo_core::tts::pocket::PocketEngine::load(&m)?;
            Ok(Box::new(e) as Box<dyn TtsEngine>)
        });

        let app_for_jobs = app.clone();
        let jobs = JobManager::new(
            platform.power.clone(),
            Arc::new(move |i: &JobInfo| {
                let (state, error) = match &i.state {
                    JobState::Queued => ("queued", None),
                    JobState::Running => ("running", None),
                    JobState::Done => ("done", None),
                    JobState::Failed(m) => ("failed", Some(m.clone())),
                    JobState::Cancelled => ("cancelled", None),
                };
                let _ = JobChangedEvent {
                    id: i.id as u32,
                    engine: engine_str(i.engine).into(),
                    label: i.label.clone(),
                    state: state.into(),
                    error,
                }
                .emit(&app_for_jobs);
            }),
        );

        let state = Arc::new(Self {
            paths,
            models,
            settings: Mutex::new(settings),
            workspace: Mutex::new(workspace),
            platform,
            host,
            stt,
            tts,
            jobs,
            recorder: Mutex::new(None),
            imports: Mutex::new(HashMap::new()),
            next_import: Mutex::new(1),
            ticker: Arc::new((Mutex::new(TickerGate::default()), Condvar::new())),
            quitting: AtomicBool::new(false),
            seen: Mutex::new(Default::default()),
        });
        // memory pressure → unload idle engines; critical also pauses the queue
        let jobs_ptr = Arc::downgrade(&state);
        state
            .host
            .watch_memory_pressure(&*state.platform.memory, move |critical| {
                if let Some(s) = jobs_ptr.upgrade() {
                    s.jobs.set_paused(critical);
                }
            });
        state
    }

    pub fn recordings_dir(&self) -> PathBuf {
        self.paths.recordings()
    }

    pub fn new_import_id(&self) -> String {
        let mut n = self.next_import.lock().unwrap();
        let id = format!("imp-{}", *n);
        *n += 1;
        id
    }
}
