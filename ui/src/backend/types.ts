// The UI talks to the core only through this interface. `real.ts` implements it over the generated Tauri bindings;
// `mock.ts` implements it with scripted fake engines so the UI runs in a plain browser (VITE_MOCK_BACKEND=1).
import type {
  AppInfo,
  AudioExportFormat,
  ImportResultDto,
  InputDeviceDto,
  IpcError,
  JobChangedEvent,
  MenuActionEvent,
  MicPermission,
  ModelsStateEvent,
  QuitRequestedEvent,
  RecorderStateEvent,
  RecorderWarningEvent,
  RecoverableDto,
  RecordingInfoDto,
  StorageEntryDto,
  SettingsDto,
  SettingsPatch,
  SttEventDto,
  TranscribeRequest,
  TranscriptExportFormat,
  TranscriptMeta,
  TtsEventDto,
  TtsRequest,
  VoiceDto,
  WorkspaceDto,
  WorkspacePatch,
} from '../bindings'

export type Unlisten = () => void

/** Typed events the core pushes (no polling from the UI). */
export interface BackendEvents {
  'recorder:state': RecorderStateEvent
  'recorder:warning': RecorderWarningEvent
  'models:state': ModelsStateEvent
  'jobs:changed': JobChangedEvent
  'menu:action': MenuActionEvent
  'quit:requested': QuitRequestedEvent
}

export type FilePickKind = 'media' | 'document' | 'any'

export interface Backend {
  readonly isMock: boolean

  appInfo(): Promise<AppInfo>
  settingsGet(): Promise<SettingsDto>
  settingsUpdate(patch: SettingsPatch): Promise<SettingsDto>

  micStatus(): Promise<MicPermission>
  micRequest(): Promise<MicPermission>
  openPrivacySettings(kind: 'microphone'): Promise<void>
  inputDevices(): Promise<InputDeviceDto[]>

  recordingStart(deviceId?: string | null): Promise<RecordingInfoDto>
  recordingPause(): Promise<void>
  recordingResume(): Promise<void>
  recordingStop(): Promise<RecordingInfoDto>
  recordingDiscard(id: string): Promise<void>

  recoverables(): Promise<RecoverableDto[]>
  recoverableDiscard(kind: 'recording' | 'stt_job' | 'tts_job', id: string): Promise<void>

  /** Starts a transcription; `onEvent` receives the stage/progress/paragraph/done/error stream. Resolves with the job id. */
  transcribe(req: TranscribeRequest, onEvent: (e: SttEventDto) => void): Promise<number>
  sttResume(jobId: string, onEvent: (e: SttEventDto) => void): Promise<number>
  jobCancel(jobId: number): Promise<void>

  tts(req: TtsRequest, onEvent: (e: TtsEventDto) => void): Promise<number>
  ttsResume(jobId: string, onEvent: (e: TtsEventDto) => void): Promise<number>
  voices(): Promise<VoiceDto[]>

  importText(path: string): Promise<ImportResultDto>
  importFull(importId: string): Promise<string>

  exportTts(outputId: string, format: AudioExportFormat, dest: string): Promise<string>
  exportTranscript(text: string, format: TranscriptExportFormat, dest: string, meta: TranscriptMeta): Promise<string>
  exportRecording(recordingId: string, format: AudioExportFormat, dest: string): Promise<string>

  workspaceGet(): Promise<WorkspaceDto>
  workspaceUpdate(patch: WorkspacePatch): Promise<void>

  storageUsage(): Promise<StorageEntryDto[]>
  /** `outputs` (generated speech) or `cache` (decode cache); never touches recordings that were not transcribed. */
  storageClear(kind: 'outputs' | 'cache'): Promise<StorageEntryDto[]>

  reveal(path: string): Promise<void>
  /** Tell the core whether the UI is visible (level-meter events only flow while recording AND visible). */
  setVisible(visible: boolean): Promise<void>
  diagnostics(): Promise<string>
  quitConfirmed(): Promise<void>

  on<K extends keyof BackendEvents>(event: K, cb: (payload: BackendEvents[K]) => void): Promise<Unlisten>

  // --- native shell helpers (dialogs, asset URLs, drag-drop) ---
  pickFile(kind: FilePickKind): Promise<string | null>
  saveFile(defaultName: string, extensions: string[]): Promise<string | null>
  /** URL the `<audio>` element can play for an absolute file path (asset protocol in the app). */
  fileUrl(path: string): string
  onFileDrop(cb: (paths: string[]) => void, onHover?: (over: boolean) => void): Promise<Unlisten>
  copyText(text: string): Promise<void>
}

/** Error thrown by every backend call that fails; mirrors the Rust `IpcError`. */
export class BackendError extends Error {
  code: string
  detail?: string | null
  constructor(e: IpcError) {
    super(e.message)
    this.code = e.code
    this.detail = e.detail
  }
}
