//! IPC contract (BRIEF §5.4). Every type here derives `specta::Type`, so `ui/src/bindings.ts` is *generated* from
//! these definitions (`cargo test -p parlo-app export_bindings`) — the TypeScript side never re-declares them.

use parlo_core::error::{DecodeError, Error};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri_specta::Event;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct IpcError {
    /// machine-readable: `mic_permission`, `disk_space`, `decode`, `cancelled`, `model_missing`, `busy`, `invalid`, `io`, `other`
    pub code: String,
    /// human-readable, shown in the UI
    pub message: String,
    pub detail: Option<String>,
}

impl IpcError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            detail: None,
        }
    }
}

impl From<Error> for IpcError {
    fn from(e: Error) -> Self {
        let message = e.to_string();
        let code = match &e {
            Error::MicPermission => "mic_permission",
            Error::DiskSpace { .. } => "disk_space",
            Error::Cancelled => "cancelled",
            Error::Decode(DecodeError::Unsupported(_)) | Error::Decode(_) => "decode",
            Error::Io { .. } | Error::IoBare(_) => "io",
            Error::State(_) => "invalid",
            Error::Other(m) if m.contains("model") && m.contains("not found") => "model_missing",
            _ => "other",
        };
        Self {
            code: code.into(),
            message,
            detail: Some(format!("{e:?}")),
        }
    }
}

pub type IpcResult<T> = Result<T, IpcError>;

// ---------------------------------------------------------------------------------------------------- app info
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ModelFileInfo {
    pub name: String,
    pub path: String,
    pub present: bool,
    #[specta(type = specta_typescript::Number)]
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct AppInfo {
    pub version: String,
    pub stt_engine: String,
    pub tts_engine: String,
    pub models: Vec<ModelFileInfo>,
    pub data_dir: String,
    pub recordings_dir: String,
    pub logs_dir: String,
    pub platform: String,
}

// ---------------------------------------------------------------------------------------------------- settings
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    System,
    Light,
    Dark,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Performance {
    Balanced,
    Maximum,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Retention {
    Day1,
    Days7,
    Days30,
    Forever,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IdleUnload {
    Immediately,
    Secs60,
    Mins5,
    Mins15,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SettingsDto {
    pub appearance: Appearance,
    pub model_idle_timeout: IdleUnload,
    pub performance: Performance,
    pub input_device: Option<String>,
    pub recording_retention: Retention,
    pub default_language: String,
    pub default_voice: String,
    #[specta(type = specta_typescript::Number)]
    pub default_speed: f32,
    pub tts_quality: String,
    /// "Allow downloads from URLs" (default on). Off hides the URL card and makes the download commands refuse.
    pub allow_downloads: bool,
    /// override for the system yt-dlp (file or directory)
    pub downloader_path: Option<String>,
    pub use_browser_cookies: bool,
    /// yt-dlp browser name (`chrome`, `firefox`, `safari`, …)
    pub cookies_browser: Option<String>,
    pub impersonate: bool,
    pub rate_limit_sleep: bool,
}

/// Partial update: fields that are `null` are left unchanged.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(default)]
pub struct SettingsPatch {
    pub appearance: Option<Appearance>,
    pub model_idle_timeout: Option<IdleUnload>,
    pub performance: Option<Performance>,
    /// `""` = follow the system default; any other value pins that device id
    pub input_device: Option<String>,
    pub recording_retention: Option<Retention>,
    pub default_language: Option<String>,
    pub default_voice: Option<String>,
    #[specta(type = Option<specta_typescript::Number>)]
    pub default_speed: Option<f32>,
    pub tts_quality: Option<String>,
    pub allow_downloads: Option<bool>,
    /// `""` = search the usual folders; any other value is the path of a yt-dlp (or a folder containing it)
    pub downloader_path: Option<String>,
    pub use_browser_cookies: Option<bool>,
    /// `""` = none
    pub cookies_browser: Option<String>,
    pub impersonate: Option<bool>,
    pub rate_limit_sleep: Option<bool>,
}

// ---------------------------------------------------------------------------------------------------- audio devices / mic
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MicPermission {
    NotDetermined,
    Granted,
    Denied,
    Restricted,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct InputDeviceDto {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

// ---------------------------------------------------------------------------------------------------- recording
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct GapDto {
    #[specta(type = specta_typescript::Number)]
    pub at_ms: u64,
    pub reason: String,
    #[specta(type = specta_typescript::Number)]
    pub gap_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct RecordingInfoDto {
    pub id: String,
    pub wav_path: String,
    pub sample_rate: u32,
    #[specta(type = specta_typescript::Number)]
    pub duration_ms: u64,
    pub started_at: String,
    pub state: String,
    pub gaps: Vec<GapDto>,
    pub device: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct RecorderStateEvent {
    /// `idle` | `recording` | `paused`
    pub state: String,
    pub id: Option<String>,
    #[specta(type = specta_typescript::Number)]
    pub elapsed_ms: u64,
    pub device: String,
    #[specta(type = specta_typescript::Number)]
    pub rms: f32,
    #[specta(type = specta_typescript::Number)]
    pub peak: f32,
    pub gaps: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecorderWarningDto {
    DeviceChanged {
        from: String,
        to: String,
    },
    PinnedDeviceLost {
        pinned: String,
        fallback: String,
    },
    StreamRestarted {
        reason: String,
    },
    ResumedAfterSleep {
        #[specta(type = specta_typescript::Number)]
        gap_ms: u64,
    },
    LowDisk {
        #[specta(type = specta_typescript::Number)]
        free_mb: u64,
    },
    SilentInput,
    Clipping,
    Stopped {
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct RecorderWarningEvent {
    pub warning: RecorderWarningDto,
}

// ---------------------------------------------------------------------------------------------------- models / jobs
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct ModelsStateEvent {
    /// `stt` | `tts`
    pub engine: String,
    /// `unloaded` | `loading` | `ready`
    pub state: String,
    #[specta(type = Option<specta_typescript::Number>)]
    pub unload_in_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct JobChangedEvent {
    pub id: u32,
    pub engine: String,
    pub label: String,
    /// `queued` | `running` | `done` | `failed` | `cancelled`
    pub state: String,
    pub error: Option<String>,
}

/// Menu items and shortcuts are forwarded to the UI as one event (`menu:<id>` semantics in a typed form).
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct MenuActionEvent {
    pub action: String,
}

/// Emitted when ⌘Q (or the Quit menu) is requested while the window is open: the UI shows its own confirmation.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct QuitRequestedEvent {
    pub recording: bool,
    pub job_running: bool,
}

// ---------------------------------------------------------------------------------------------------- transcription
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SttSource {
    Path {
        path: String,
    },
    Recording {
        recording_id: String,
    },
    /// download the audio of this link first (http/https), then transcribe it
    Url {
        url: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct TranscribeRequest {
    pub source: SttSource,
    /// "auto" or an ISO-639-1 code
    pub language: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct TranscriptDto {
    pub text: String,
    pub language: Option<String>,
    #[specta(type = Option<specta_typescript::Number>)]
    pub language_prob: Option<f32>,
    #[specta(type = specta_typescript::Number)]
    pub duration_s: f64,
    #[specta(type = specta_typescript::Number)]
    pub speech_s: f64,
    pub words: u32,
    #[specta(type = specta_typescript::Number)]
    pub processing_s: f64,
    pub source_name: String,
    pub recording_id: Option<String>,
    /// the link this was downloaded from (URL jobs)
    pub source_url: Option<String>,
    /// directory id of the kept download (`dl-…`); the audio stays until this transcript is replaced or cleared
    pub download_id: Option<String>,
    /// the downloaded audio file (player, re-transcribe, export)
    pub media_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SttEventDto {
    /// `downloading` (URL jobs only) | `preparing` | `detecting_speech` | `detecting_language` | `loading_model` | `transcribing`
    Stage {
        stage: String,
    },
    /// title (and length) of the link, known before the first byte arrives
    DownloadInfo {
        title: String,
        #[specta(type = Option<specta_typescript::Number>)]
        duration_s: Option<f64>,
        /// the link was a playlist: only its first item is transcribed
        playlist: bool,
    },
    /// yt-dlp's rate-limit sleep before the download starts ("Waiting N s")
    DownloadWaiting {
        #[specta(type = specta_typescript::Number)]
        secs: f64,
    },
    DownloadProgress {
        #[specta(type = specta_typescript::Number)]
        downloaded_bytes: u64,
        #[specta(type = Option<specta_typescript::Number>)]
        total_bytes: Option<u64>,
        #[specta(type = Option<specta_typescript::Number>)]
        speed_bps: Option<f64>,
        #[specta(type = Option<specta_typescript::Number>)]
        eta_s: Option<f64>,
    },
    /// the first downloader failed and another one took over ("Bundled downloader failed, retried with system yt-dlp")
    DownloaderFallback {
        /// `bundled` | `system` | `curl`
        from: String,
        to: String,
        message: String,
    },
    /// the audio is complete and on disk; transcription starts
    Downloaded {
        download_id: String,
        media_path: String,
        title: String,
        #[specta(type = specta_typescript::Number)]
        bytes: u64,
    },
    StageProgress {
        #[specta(type = specta_typescript::Number)]
        fraction: f64,
    },
    Language {
        code: String,
        #[specta(type = specta_typescript::Number)]
        prob: f32,
    },
    Progress {
        #[specta(type = specta_typescript::Number)]
        done_s: f64,
        #[specta(type = specta_typescript::Number)]
        total_s: f64,
        #[specta(type = Option<specta_typescript::Number>)]
        eta_ms: Option<u64>,
    },
    Paragraph {
        index: u32,
        text: String,
        #[specta(type = specta_typescript::Number)]
        start_s: f64,
        #[specta(type = specta_typescript::Number)]
        end_s: f64,
    },
    Done {
        transcript: TranscriptDto,
    },
    Error {
        error: IpcError,
    },
}

// ---------------------------------------------------------------------------------------------------- TTS
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TtsSource {
    Text { text: String },
    Import { import_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct TtsRequest {
    pub source: TtsSource,
    pub voice: String,
    /// "standard" | "fast"
    pub quality: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TtsEventDto {
    Stage {
        stage: String,
    },
    Progress {
        done_chars: u32,
        total_chars: u32,
        #[specta(type = Option<specta_typescript::Number>)]
        eta_ms: Option<u64>,
    },
    SegmentReady {
        index: u32,
        path: String,
        #[specta(type = specta_typescript::Number)]
        duration_ms: u64,
    },
    Done {
        output_id: String,
        #[specta(type = specta_typescript::Number)]
        duration_ms: u64,
        segments: Vec<String>,
        cache_hit: bool,
    },
    Error {
        error: IpcError,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct VoiceDto {
    pub id: String,
    pub label: String,
    /// ISO-639-1 language the voice speaks (`en`, `es`)
    pub language: String,
    /// absolute path of the bundled preview clip (play with the asset protocol), if present
    pub preview_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ImportResultDto {
    pub import_id: String,
    pub file_name: String,
    pub kind: String,
    /// the full text, unless the document is large (then use `preview` and `text_import_full`)
    pub text: Option<String>,
    pub preview: String,
    pub large: bool,
    pub chars: u32,
    pub words: u32,
    pub pages: Option<u32>,
    pub chapters: Option<u32>,
    #[specta(type = specta_typescript::Number)]
    pub est_minutes: f64,
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------------------------------- export / workspace / recovery
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum AudioExportFormat {
    Wav,
    M4a,
    Mp3,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptExportFormat {
    Txt,
    Md,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct TranscriptMeta {
    pub source_name: String,
    pub language: Option<String>,
    #[specta(type = specta_typescript::Number)]
    pub duration_s: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct WorkspaceTranscriptDto {
    pub text: String,
    pub source_name: String,
    pub language: Option<String>,
    #[specta(type = specta_typescript::Number)]
    pub duration_s: f64,
    pub words: u32,
    pub edited: bool,
    pub exported_or_copied: bool,
    pub recording_id: Option<String>,
    pub source_url: Option<String>,
    pub download_id: Option<String>,
    /// filled in by the core when the kept download still exists (ignored on update)
    pub media_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct WorkspaceTtsDto {
    pub text: String,
    pub voice: String,
    pub output_id: Option<String>,
    pub source_name: Option<String>,
    /// segment paths of the current output (if it still exists), so the player can be rebuilt on launch
    pub segments: Vec<String>,
    #[specta(type = Option<specta_typescript::Number>)]
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct WorkspaceDto {
    pub transcript: Option<WorkspaceTranscriptDto>,
    pub tts: Option<WorkspaceTtsDto>,
    /// an unfinished download from a previous run: refill the URL field with it
    pub pending_url: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(default)]
pub struct WorkspacePatch {
    pub transcript: Option<WorkspaceTranscriptDto>,
    pub tts: Option<WorkspaceTtsDto>,
    pub clear_transcript: bool,
    pub clear_tts: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecoverableDto {
    Recording {
        id: String,
        wav_path: String,
        #[specta(type = specta_typescript::Number)]
        duration_ms: u64,
        started_at: String,
        /// stopped and saved by the quit dialog (nothing lost; just never transcribed)
        saved_on_quit: bool,
    },
    SttJob {
        job_id: String,
        source_name: String,
    },
    TtsJob {
        job_id: String,
        text_preview: String,
        voice: String,
    },
}

// ---------------------------------------------------------------------------------------------------- storage
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct StorageEntryDto {
    /// `recordings` | `outputs` | `cache` | `models`
    pub kind: String,
    pub label: String,
    #[specta(type = specta_typescript::Number)]
    pub bytes: u64,
    pub path: String,
}

// ---------------------------------------------------------------------------------------------------- downloads
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DownloaderToolDto {
    /// `bundled` | `system`
    pub kind: String,
    pub path: String,
    pub version: Option<String>,
    /// can this build impersonate Chrome (`--impersonate chrome`)? `None` = unknown
    pub impersonate: Option<bool>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct BrowserDto {
    /// yt-dlp's name (`chrome`, `firefox`, `safari`, …)
    pub id: String,
    pub label: String,
}

/// What Settings → Downloads shows: the downloader(s) in use and the browsers whose cookies could be read.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DownloadsInfoDto {
    pub tools: Vec<DownloaderToolDto>,
    pub curl: Option<String>,
    /// a system deno (JavaScript runtime for YouTube), if found
    pub deno: Option<String>,
    pub browsers: Vec<BrowserDto>,
}
