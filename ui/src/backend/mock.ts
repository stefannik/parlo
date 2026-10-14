// Mock backend (VITE_MOCK_BACKEND=1): fake engines and recorder with scripted progress, fake levels and sample outputs,
// so the whole UI can run — and be screenshotted — in a plain browser. It mirrors the real IPC contract; see
// `contract.test.ts`, which runs the same assertions against this implementation.
import type {
  AppInfo,
  AudioExportFormat,
  ImportResultDto,
  InputDeviceDto,
  MicPermission,
  RecorderStateEvent,
  RecordingInfoDto,
  RecoverableDto,
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
import { BackendError, type Backend, type BackendEvents, type FilePickKind, type Unlisten } from './types'

export interface MockOptions {
  /** multiplies every scripted delay (0 = instant; used by tests) */
  timeScale?: number
  micPermission?: MicPermission
  /** start with recoverable items listed */
  recoverables?: RecoverableDto[]
  /** pre-existing workspace (restored transcript / TTS output) */
  workspace?: WorkspaceDto
  /** make the next transcription / generation fail (once) */
  failNextStt?: boolean
  failNextTts?: boolean
}

/** Gate points a script can be held at (used by the screenshot tool): `window.__parloMock.hold('stt:transcribing')`. */
export interface MockControl {
  hold(point: string): void
  release(): void
  held: string | null
  options: MockOptions
}

const VOICES: Array<[string, string]> = [
  ['george', 'George'],
  ['alba', 'Alba'],
  ['anna', 'Anna'],
  ['charles', 'Charles'],
  ['cosette', 'Cosette'],
  ['jane', 'Jane'],
  ['mary', 'Mary'],
  ['michael', 'Michael'],
  ['paul', 'Paul'],
  ['vera', 'Vera'],
]

const SAMPLE_PARAGRAPHS = [
  "Good morning everyone, and thanks for joining. Today I'd like to walk through the plan for the next quarter, starting with what we shipped and what we learned from it.",
  'The first thing to say is that the new onboarding flow worked better than we expected. Activation went up by eleven percent, and support tickets about setup dropped by almost a third.',
  "On the other hand, the export feature is still too slow for large projects. We've profiled it, and most of the time is spent re-encoding audio that didn't change, so the fix should be fairly contained.",
  "Finally, a note on priorities. If we have to cut something, I'd rather cut the dashboard redesign than delay the offline mode, because offline mode is the thing people keep asking about.",
  "That's all I have. Let's open it up for questions.",
]

const wait = (ms: number) => new Promise<void>((r) => setTimeout(r, ms))

export class MockBackend implements Backend {
  readonly isMock = true
  readonly control: MockControl
  private listeners = new Map<string, Set<(p: unknown) => void>>()
  private settings: SettingsDto = {
    appearance: 'system',
    model_idle_timeout: 'mins5',
    performance: 'balanced',
    input_device: null,
    recording_retention: 'days7',
    default_language: 'auto',
    default_voice: 'george',
    default_speed: 1.0,
    tts_quality: 'standard',
  }
  private workspace: WorkspaceDto
  private recoverableList: RecoverableDto[]
  private mic: MicPermission
  private rec: { id: string; startedAt: number; accumulated: number; resumedAt: number | null; timer: ReturnType<typeof setInterval> | null } | null = null
  private recState: 'idle' | 'recording' | 'paused' = 'idle'
  private cancelled = new Set<number>()
  private nextJob = 1
  private gate: { point: string; resolve: () => void } | null = null
  private imports = new Map<string, string>()
  private dropCb: ((paths: string[]) => void) | null = null
  private hoverCb: ((over: boolean) => void) | null = null
  private visible = true
  private opts: MockOptions

  constructor(opts: MockOptions = {}) {
    this.opts = opts
    this.mic = opts.micPermission ?? 'granted'
    this.recoverableList = opts.recoverables ? [...opts.recoverables] : []
    this.workspace = opts.workspace ?? {}
    const self = this
    this.control = {
      held: null,
      options: this.opts,
      hold(point: string) {
        this.held = point
      },
      release() {
        this.held = null
        self.gate?.resolve()
        self.gate = null
      },
    }
  }

  private scale(ms: number) {
    return ms * (this.opts.timeScale ?? 1)
  }

  private async sleep(ms: number) {
    const s = this.scale(ms)
    if (s > 0) await wait(s)
  }

  /** Scripts call this at named points; if the control is holding that point they block until `release()`. */
  private async checkpoint(point: string) {
    if (this.control.held === point) {
      await new Promise<void>((resolve) => {
        this.gate = { point, resolve }
      })
    }
  }

  private emit<K extends keyof BackendEvents>(event: K, payload: BackendEvents[K]) {
    this.listeners.get(event)?.forEach((cb) => cb(payload))
  }

  /** Test/screenshot hook: simulate an OS file drop or a menu action. */
  simulateDrop(paths: string[]) {
    this.dropCb?.(paths)
  }
  simulateHover(over: boolean) {
    this.hoverCb?.(over)
  }
  simulateMenu(action: string) {
    this.emit('menu:action', { action })
  }
  simulateQuit() {
    this.emit('quit:requested', { recording: this.recState !== 'idle', job_running: false })
  }
  emitWarning(w: BackendEvents['recorder:warning']['warning']) {
    this.emit('recorder:warning', { warning: w })
  }
  emitModels(engine: 'stt' | 'tts', state: 'unloaded' | 'loading' | 'ready', unload_in_ms: number | null = null) {
    this.emit('models:state', { engine, state, unload_in_ms })
  }

  // ------------------------------------------------------------------------------------------------------ app
  async appInfo(): Promise<AppInfo> {
    return {
      version: '0.1.0',
      stt_engine: 'whisper.cpp · large-v3-turbo (q8_0)',
      tts_engine: 'Kyutai PocketTTS 2.1.0 (Candle)',
      models: [
        { name: 'ggml-large-v3-turbo-q8_0.bin', path: '/mock/models/ggml-large-v3-turbo-q8_0.bin', present: true, size_bytes: 874_000_000 },
        { name: 'ggml-silero-v5.1.2.bin', path: '/mock/models/ggml-silero-v5.1.2.bin', present: true, size_bytes: 885_000 },
        { name: 'pocket-tts 2.1.0', path: '/mock/models/pockettts', present: true, size_bytes: 480_000_000 },
      ],
      data_dir: '/mock/Parlo',
      recordings_dir: '/mock/Parlo/recordings',
      logs_dir: '/mock/Parlo/logs',
      platform: 'mock',
    }
  }
  async settingsGet() {
    return { ...this.settings }
  }
  async settingsUpdate(patch: SettingsPatch) {
    const { input_device, ...rest } = patch
    this.settings = { ...this.settings, ...stripUndefined(stripNull(rest)) }
    if (input_device != null) this.settings.input_device = input_device === '' ? null : input_device
    return { ...this.settings }
  }

  // ------------------------------------------------------------------------------------------------------ mic
  async micStatus() {
    return this.mic
  }
  async micRequest() {
    if (this.mic === 'not_determined') this.mic = 'granted'
    return this.mic
  }
  async openPrivacySettings() {}
  async inputDevices(): Promise<InputDeviceDto[]> {
    return [
      { id: 'mbp-mic', name: 'MacBook Pro Microphone', is_default: true },
      { id: 'airpods', name: "Stefan's AirPods Pro", is_default: false },
      { id: 'scarlett', name: 'Scarlett 2i2 USB', is_default: false },
    ]
  }

  // ------------------------------------------------------------------------------------------------------ recording
  private elapsed() {
    const r = this.rec
    if (!r) return 0
    return r.accumulated + (r.resumedAt != null ? Date.now() - r.resumedAt : 0)
  }
  private pushState(rms = 0, peak = 0) {
    const r = this.rec
    const ev: RecorderStateEvent = {
      state: this.recState,
      id: r?.id ?? null,
      elapsed_ms: this.elapsed(),
      device: this.settings.input_device === 'airpods' ? "Stefan's AirPods Pro" : 'MacBook Pro Microphone',
      rms,
      peak,
      gaps: 0,
    }
    this.emit('recorder:state', ev)
  }
  private info(): RecordingInfoDto {
    const r = this.rec!
    return {
      id: r.id,
      wav_path: `/mock/recordings/${r.id}.wav`,
      sample_rate: 48000,
      duration_ms: this.elapsed(),
      started_at: new Date(r.startedAt).toISOString(),
      state: this.recState,
      gaps: [],
      device: 'MacBook Pro Microphone',
    }
  }
  async recordingStart() {
    if (this.mic === 'denied' || this.mic === 'restricted') throw new BackendError({ code: 'mic_permission', message: 'Microphone access is turned off for Parlo.', detail: null })
    if (this.recState !== 'idle') throw new BackendError({ code: 'busy', message: 'Already recording', detail: null })
    this.mic = 'granted'
    const id = `rec-${Date.now()}`
    this.rec = { id, startedAt: Date.now(), accumulated: 0, resumedAt: Date.now(), timer: null }
    this.recState = 'recording'
    this.pushState()
    // fake levels, only while recording and visible (the real ticker does the same)
    let phase = 0
    this.rec.timer = setInterval(() => {
      if (!this.visible || this.recState !== 'recording') return
      phase += 0.35
      const peak = Math.min(0.95, Math.abs(Math.sin(phase) * 0.45 + Math.sin(phase * 2.7) * 0.25) + 0.05)
      this.pushState(peak * 0.55, peak)
    }, 100)
    return this.info()
  }
  async recordingPause() {
    const r = this.rec
    if (!r || this.recState !== 'recording') return
    r.accumulated += Date.now() - (r.resumedAt ?? Date.now())
    r.resumedAt = null
    this.recState = 'paused'
    this.pushState()
  }
  async recordingResume() {
    const r = this.rec
    if (!r || this.recState !== 'paused') return
    r.resumedAt = Date.now()
    this.recState = 'recording'
    this.pushState()
  }
  private endRecording() {
    const r = this.rec
    if (r?.timer) clearInterval(r.timer)
    this.rec = null
    this.recState = 'idle'
    this.emit('recorder:state', { state: 'idle', id: null, elapsed_ms: 0, device: '', rms: 0, peak: 0, gaps: 0 })
  }
  async recordingStop() {
    if (!this.rec) throw new BackendError({ code: 'invalid', message: 'Not recording', detail: null })
    if (this.recState === 'recording') await this.recordingPause()
    const info = this.info()
    this.endRecording()
    return info
  }
  async recordingDiscard(_id: string) {
    if (this.rec) this.endRecording()
  }

  // ------------------------------------------------------------------------------------------------------ recovery
  async recoverables() {
    return [...this.recoverableList]
  }
  async recoverableDiscard(kind: 'recording' | 'stt_job' | 'tts_job', id: string) {
    this.recoverableList = this.recoverableList.filter((r) => {
      if (kind === 'recording') return !(r.kind === 'recording' && r.id === id)
      return !((r.kind === 'stt_job' || r.kind === 'tts_job') && r.job_id === id)
    })
  }

  // ------------------------------------------------------------------------------------------------------ STT
  async transcribe(req: TranscribeRequest, onEvent: (e: SttEventDto) => void) {
    const job = this.nextJob++
    void this.runStt(job, req, onEvent)
    return job
  }
  async sttResume(jobId: string, onEvent: (e: SttEventDto) => void) {
    const rec = this.recoverableList.find((r) => r.kind === 'stt_job' && r.job_id === jobId)
    await this.recoverableDiscard('stt_job', jobId)
    return this.transcribe(
      { source: { kind: 'path', path: `/mock/${rec && rec.kind === 'stt_job' ? rec.source_name : 'resumed.m4a'}` }, language: 'auto' },
      onEvent,
    )
  }
  private async runStt(job: number, req: TranscribeRequest, onEvent: (e: SttEventDto) => void) {
    const name = req.source.kind === 'path' ? req.source.path.split('/').pop()! : 'Recording'
    const total = 252.4
    const stage = async (s: string, ms: number, steps = 4) => {
      onEvent({ type: 'stage', stage: s })
      if (s === 'loading_model') this.emitModels('stt', 'loading')
      for (let i = 1; i <= steps; i++) {
        if (this.cancelled.has(job)) return false
        await this.sleep(ms / steps)
        onEvent({ type: 'stage_progress', fraction: i / steps })
      }
      return true
    }
    if (this.opts.failNextStt) {
      this.opts.failNextStt = false
      onEvent({ type: 'stage', stage: 'preparing' })
      await this.sleep(300)
      onEvent({ type: 'error', error: { code: 'decode', message: `Parlo couldn't read “${name}”. The file may be damaged or in a format it doesn't support.`, detail: 'decode error: unsupported codec (mock)' } })
      return
    }
    await this.checkpoint('stt:preparing')
    if (!(await stage('preparing', 700))) return this.finishCancel(job, onEvent)
    await this.checkpoint('stt:detecting_speech')
    if (!(await stage('detecting_speech', 600))) return this.finishCancel(job, onEvent)
    if (req.language === 'auto') {
      await this.checkpoint('stt:detecting_language')
      if (!(await stage('detecting_language', 500, 2))) return this.finishCancel(job, onEvent)
    }
    const code = req.language === 'auto' ? 'en' : req.language
    onEvent({ type: 'language', code, prob: req.language === 'auto' ? 0.97 : 1 })
    await this.checkpoint('stt:loading_model')
    if (!(await stage('loading_model', 800, 2))) return this.finishCancel(job, onEvent)
    this.emitModels('stt', 'ready', 300_000)
    onEvent({ type: 'stage', stage: 'transcribing' })
    const texts: string[] = []
    let t = 0
    for (let i = 0; i < SAMPLE_PARAGRAPHS.length; i++) {
      await this.sleep(1100)
      if (this.cancelled.has(job)) return this.finishCancel(job, onEvent)
      const len = total / SAMPLE_PARAGRAPHS.length
      const text = SAMPLE_PARAGRAPHS[i]!
      texts.push(text)
      onEvent({ type: 'paragraph', index: i, text, start_s: t, end_s: t + len })
      t += len
      onEvent({ type: 'progress', done_s: t, total_s: total, eta_ms: Math.round((total - t) * 40) })
      if (i === 1) await this.checkpoint('stt:transcribing')
    }
    const full = texts.join('\n\n')
    onEvent({
      type: 'done',
      transcript: {
        text: full,
        language: code,
        language_prob: 0.97,
        duration_s: total,
        speech_s: 231.2,
        words: full.split(/\s+/).length,
        processing_s: 6.1,
        source_name: name,
        recording_id: req.source.kind === 'recording' ? req.source.recording_id : null,
      },
    })
    this.emitModels('stt', 'ready', 300_000)
  }
  private finishCancel(job: number, onEvent: (e: SttEventDto) => void) {
    this.cancelled.delete(job)
    onEvent({ type: 'error', error: { code: 'cancelled', message: 'Cancelled', detail: null } })
  }
  async jobCancel(jobId: number) {
    this.cancelled.add(jobId)
  }

  // ------------------------------------------------------------------------------------------------------ TTS
  async voices(): Promise<VoiceDto[]> {
    return VOICES.map(([id, label]) => ({ id, label, preview_path: `/mock/previews/${id}.mp3` }))
  }
  async tts(req: TtsRequest, onEvent: (e: TtsEventDto) => void) {
    const job = this.nextJob++
    void this.runTts(job, req, onEvent)
    return job
  }
  async ttsResume(jobId: string, onEvent: (e: TtsEventDto) => void) {
    const rec = this.recoverableList.find((r) => r.kind === 'tts_job' && r.job_id === jobId)
    await this.recoverableDiscard('tts_job', jobId)
    return this.tts({ source: { kind: 'text', text: rec && rec.kind === 'tts_job' ? rec.text_preview : 'Resumed text.' }, voice: 'george', quality: 'standard' }, onEvent)
  }
  private async runTts(job: number, req: TtsRequest, onEvent: (e: TtsEventDto) => void) {
    const text = req.source.kind === 'text' ? req.source.text : (this.imports.get(req.source.import_id) ?? '')
    const total = Math.max(1, text.length)
    if (this.opts.failNextTts) {
      this.opts.failNextTts = false
      onEvent({ type: 'stage', stage: 'preparing' })
      await this.sleep(300)
      onEvent({ type: 'error', error: { code: 'model_missing', message: 'The speech model files are missing. Reinstall Parlo to restore them.', detail: 'model not found: pockettts (mock)' } })
      return
    }
    onEvent({ type: 'stage', stage: 'loading_model' })
    this.emitModels('tts', 'loading')
    await this.sleep(700)
    this.emitModels('tts', 'ready', 300_000)
    onEvent({ type: 'stage', stage: 'generating' })
    const words = text.split(/\s+/).filter(Boolean).length
    const totalMs = Math.max(4000, Math.round((words / 150) * 60_000))
    const segCount = Math.max(1, Math.min(6, Math.ceil(totalMs / 25_000)))
    const segments: string[] = []
    const id = `out-${hash(text + req.voice)}`
    for (let i = 0; i < segCount; i++) {
      for (let s = 1; s <= 4; s++) {
        await this.sleep(300)
        if (this.cancelled.has(job)) {
          this.cancelled.delete(job)
          return onEvent({ type: 'error', error: { code: 'cancelled', message: 'Cancelled', detail: null } })
        }
        const frac = (i + s / 4) / segCount
        onEvent({ type: 'progress', done_chars: Math.round(total * frac), total_chars: total, eta_ms: Math.round((1 - frac) * segCount * 1200) })
      }
      const path = `/mock/outputs/${id}/seg-${String(i).padStart(3, '0')}.wav`
      segments.push(path)
      onEvent({ type: 'segment_ready', index: i, path, duration_ms: Math.round(totalMs / segCount) })
      if (i === 0) await this.checkpoint('tts:generating')
    }
    onEvent({ type: 'done', output_id: id, duration_ms: totalMs, segments, cache_hit: false })
  }

  // ------------------------------------------------------------------------------------------------------ import / export
  async importText(path: string): Promise<ImportResultDto> {
    const file_name = path.split('/').pop() ?? path
    const ext = file_name.split('.').pop()?.toLowerCase() ?? 'txt'
    const text =
      'Chapter One\n\n' +
      Array.from({ length: 6 }, (_, i) => `${SAMPLE_PARAGRAPHS[i % SAMPLE_PARAGRAPHS.length]}`).join('\n\n')
    const import_id = `imp-${hash(path)}`
    this.imports.set(import_id, text)
    const words = text.split(/\s+/).length
    return {
      import_id,
      file_name,
      kind: ext,
      text,
      preview: text.slice(0, 400),
      large: false,
      chars: text.length,
      words,
      pages: ext === 'pdf' ? 12 : null,
      chapters: ext === 'epub' ? 3 : null,
      est_minutes: words / 150,
      warnings: ext === 'pdf' ? ['Removed repeating page headers and footers.'] : [],
    }
  }
  async importFull(importId: string) {
    return this.imports.get(importId) ?? ''
  }
  async exportTts(_outputId: string, format: AudioExportFormat, dest: string) {
    await this.sleep(400)
    return ensureExt(dest, format)
  }
  async exportTranscript(_text: string, format: TranscriptExportFormat, dest: string, _meta: TranscriptMeta) {
    return ensureExt(dest, format)
  }
  async exportRecording(_id: string, format: AudioExportFormat, dest: string) {
    return ensureExt(dest, format)
  }

  // ------------------------------------------------------------------------------------------------------ workspace etc.
  async workspaceGet() {
    return structuredClone(this.workspace)
  }
  async workspaceUpdate(patch: WorkspacePatch) {
    if (patch.clear_transcript) this.workspace.transcript = null
    if (patch.transcript) this.workspace.transcript = patch.transcript
    if (patch.clear_tts) this.workspace.tts = null
    if (patch.tts) this.workspace.tts = patch.tts
  }
  async reveal(_path: string) {}
  async setVisible(visible: boolean) {
    this.visible = visible
  }
  async diagnostics() {
    return 'Parlo 0.1.0 (mock)\nplatform: mock\n--- last log lines ---\n(mock backend: no logs)'
  }
  async quitConfirmed() {}

  async on<K extends keyof BackendEvents>(event: K, cb: (payload: BackendEvents[K]) => void): Promise<Unlisten> {
    let set = this.listeners.get(event)
    if (!set) this.listeners.set(event, (set = new Set()))
    set.add(cb as (p: unknown) => void)
    return () => set!.delete(cb as (p: unknown) => void)
  }

  // ------------------------------------------------------------------------------------------------------ native helpers
  async pickFile(kind: FilePickKind) {
    return kind === 'document' ? '/Users/me/Documents/meeting-notes.docx' : '/Users/me/Downloads/team-meeting.m4a'
  }
  async saveFile(defaultName: string, _extensions: string[]) {
    return `/Users/me/Desktop/${defaultName}`
  }
  fileUrl(path: string) {
    return `mock-audio://${path}`
  }
  async onFileDrop(cb: (paths: string[]) => void, onHover?: (over: boolean) => void): Promise<Unlisten> {
    this.dropCb = cb
    this.hoverCb = onHover ?? null
    return () => {
      this.dropCb = null
      this.hoverCb = null
    }
  }
  async copyText(text: string) {
    try {
      await navigator.clipboard?.writeText(text)
    } catch {
      /* clipboard unavailable in headless browsers */
    }
  }
}

function stripUndefined<T extends object>(o: T): Partial<T> {
  return Object.fromEntries(Object.entries(o).filter(([, v]) => v !== undefined)) as Partial<T>
}
function stripNull<T extends object>(o: T): Partial<T> {
  return Object.fromEntries(Object.entries(o).filter(([, v]) => v !== null)) as Partial<T>
}
function hash(s: string) {
  let h = 5381
  for (let i = 0; i < s.length; i++) h = ((h << 5) + h + s.charCodeAt(i)) | 0
  return (h >>> 0).toString(16)
}
function ensureExt(dest: string, ext: string) {
  return dest.toLowerCase().endsWith(`.${ext}`) ? dest : `${dest}.${ext}`
}
