// Mock backend (VITE_MOCK_BACKEND=1): fake engines and recorder with scripted progress, fake levels and sample outputs,
// so the whole UI can run — and be screenshotted — in a plain browser. It mirrors the real IPC contract; see
// `contract.test.ts`, which runs the same assertions against this implementation.
import type {
  AppInfo,
  AudioExportFormat,
  DownloadsInfoDto,
  ImportResultDto,
  InputDeviceDto,
  MicPermission,
  RecorderStateEvent,
  RecordingInfoDto,
  StorageEntryDto,
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
import { checkUrl, fallbackTitle, hostOf } from '../lib/url'
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
  /** start with "Allow downloads from URLs" switched off */
  downloadsOff?: boolean
}

/** Gate points a script can be held at (used by the screenshot tool): `window.__parloMock.hold('stt:transcribing')`. */
export interface MockControl {
  hold(point: string): void
  release(): void
  held: string | null
  options: MockOptions
}

// same order as the real engine (English voices alphabetical by id, then Spanish)
const VOICES: Array<[string, string, string]> = [
  ['alba', 'Alba', 'en'],
  ['anna', 'Anna', 'en'],
  ['azelma', 'Azelma', 'en'],
  ['bill_boerst', 'Bill Boerst', 'en'],
  ['caro_davy', 'Caro Davy', 'en'],
  ['charles', 'Charles', 'en'],
  ['cosette', 'Cosette', 'en'],
  ['eponine', 'Eponine', 'en'],
  ['eve', 'Eve', 'en'],
  ['fantine', 'Fantine', 'en'],
  ['george', 'George', 'en'],
  ['jane', 'Jane', 'en'],
  ['javert', 'Javert', 'en'],
  ['jean', 'Jean', 'en'],
  ['marius', 'Marius', 'en'],
  ['mary', 'Mary', 'en'],
  ['michael', 'Michael', 'en'],
  ['paul', 'Paul', 'en'],
  ['peter_yearsley', 'Peter Yearsley', 'en'],
  ['stuart_bell', 'Stuart Bell', 'en'],
  ['vera', 'Vera', 'en'],
  ['lola', 'Lola', 'es'],
]

const SAMPLE_PARAGRAPHS = [
  "Good morning everyone, and thanks for joining. Today I'd like to walk through the plan for the next quarter, starting with what we shipped and what we learned from it.",
  'The first thing to say is that the new onboarding flow worked better than we expected. Activation went up by eleven percent, and support tickets about setup dropped by almost a third.',
  "On the other hand, the export feature is still too slow for large projects. We've profiled it, and most of the time is spent re-encoding audio that didn't change, so the fix should be fairly contained.",
  "Finally, a note on priorities. If we have to cut something, I'd rather cut the dashboard redesign than delay the offline mode, because offline mode is the thing people keep asking about.",
  "That's all I have. Let's open it up for questions.",
]

const DOWNLOADS_OFF = 'Downloads from links are turned off. Turn them on in Settings → Downloads.'

/** The URL keywords the mock reacts to (the real core classifies the downloader's own error text). */
const DOWNLOAD_RULES: Array<[RegExp, string, string]> = [
  [/\/live\b|livestream/i, 'download_live', 'This is a live stream, so there is nothing to transcribe yet. Try again after it has ended.'],
  [/\/private|\/age\b|members-only/i, 'download_login', 'This video is private, members-only or age-restricted. If your browser is signed in to the site, turn on Settings → Downloads → “Use browser cookies” and try again.'],
  [/\/cookies/i, 'download_cookies', 'Couldn’t read your browser’s cookies. In Settings → Downloads, check the browser under “Use browser cookies” (close it and try again, allow the Keychain prompt, or give Parlo Full Disk Access for Safari) — or turn the option off.'],
  [/spotify\.com|\/drm/i, 'download_drm', 'This content is protected (DRM) or comes from a site that doesn’t allow downloads (like Spotify or Netflix), so Parlo can’t read it.'],
  [/\/notfound|\/404/i, 'download_not_found', 'That link doesn’t seem to exist any more (not found). Check the address and try again.'],
  [/\/offline|\/network/i, 'download_network', 'Couldn’t reach the website. Check your internet connection and try again.'],
  [/\/noaudio/i, 'download_no_audio', 'Couldn’t find an audio track at this link.'],
  [/\/diskfull/i, 'disk_space', 'Not enough free disk space to download this audio. Free some space and try again.'],
  [/\/unsupported|\/fail\b/i, 'download_failed', 'Couldn’t download this link. Install or update yt-dlp: `brew install yt-dlp` / `brew upgrade yt-dlp`.'],
]

/** Title the "site" reports for a link. */
function titleFor(url: string) {
  if (/youtube\.com|youtu\.be/.test(url)) return 'Local-first software: an interview with the authors'
  if (/\.(mp3|m4a|aac|ogg|opus|wav)(\?|$)/i.test(url)) return fallbackTitle(url)
  return `Episode 12 · ${hostOf(url)}`
}

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
    allow_downloads: true,
    downloader_path: null,
    use_browser_cookies: false,
    cookies_browser: null,
    impersonate: true,
    rate_limit_sleep: true,
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
    this.workspace = opts.workspace ?? { transcript: null, tts: null, pending_url: null, transcript_parked: false }
    if (opts.downloadsOff) this.settings.allow_downloads = false
    this.control = {
      held: null,
      options: this.opts,
      hold: (point: string) => {
        this.control.held = point
      },
      release: () => {
        this.control.held = null
        this.gate?.resolve()
        this.gate = null
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
    // null = leave unchanged; input_device "" = follow the system default
    const next: Record<string, unknown> = { ...this.settings }
    for (const [k, v] of Object.entries(patch)) {
      if (v == null) continue
      next[k] = v === '' && (k === 'input_device' || k === 'downloader_path' || k === 'cookies_browser') ? null : v
    }
    this.settings = next as unknown as SettingsDto
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
    this.durations.set(info.wav_path, info.duration_ms)
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
    if (req.source.kind === 'url') {
      // same order of checks as the core: switch → validation
      if (!this.settings.allow_downloads) throw new BackendError({ code: 'download_disabled', message: DOWNLOADS_OFF, detail: null })
      const c = checkUrl(req.source.url)
      if (!c.url) throw new BackendError({ code: 'invalid_url', message: c.problem ?? 'Invalid link', detail: null })
      this.workspace.pending_url = c.url
      req = { ...req, source: { kind: 'url', url: c.url } }
    }
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
    let name = req.source.kind === 'path' ? req.source.path.split('/').pop()! : req.source.kind === 'url' ? fallbackTitle(req.source.url) : 'Recording'
    const total = 252.4
    if (req.source.kind === 'path') this.durations.set(req.source.path, total * 1000)
    let downloaded: { id: string; path: string; url: string } | null = null
    if (req.source.kind === 'url') {
      const d = await this.runDownload(job, req.source.url, onEvent)
      if (!d) return
      name = d.title
      downloaded = { id: d.id, path: d.path, url: req.source.url }
      this.durations.set(d.path, total * 1000)
    } else if (req.source.kind === 'path' && req.source.path.includes('/mock/downloads/')) {
      // re-transcribing audio that was downloaded earlier: keep its link
      const id = req.source.path.split('/').at(-2)!
      downloaded = { id, path: req.source.path, url: this.workspace.transcript?.source_url ?? '' }
      name = this.workspace.transcript?.source_name ?? name
    }
    const stage = async (s: string, ms: number, steps = 4) => {
      onEvent({ type: 'stage', stage: s })
      if (s === 'loading_model') this.emitModels('stt', 'loading')
      await this.checkpoint(`stt:${s}`)
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
    if (!(await stage('preparing', 700))) return this.finishCancel(job, onEvent)
    if (!(await stage('detecting_speech', 600))) return this.finishCancel(job, onEvent)
    if (req.language === 'auto') {
        if (!(await stage('detecting_language', 500, 2))) return this.finishCancel(job, onEvent)
    }
    const code = req.language === 'auto' ? 'en' : req.language
    onEvent({ type: 'language', code, prob: req.language === 'auto' ? 0.97 : 1 })
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
        source_url: downloaded?.url || null,
        download_id: downloaded?.id ?? null,
        media_path: downloaded?.path ?? null,
      },
    })
    this.emitModels('stt', 'ready', 300_000)
  }
  /** Scripted download stage. Resolves with the downloaded item, or `null` after sending the terminal error event. */
  private async runDownload(job: number, url: string, onEvent: (e: SttEventDto) => void) {
    const fail = (code: string, message: string, detail: string | null = null) => {
      this.cancelled.delete(job)
      if (code === 'cancelled') this.workspace.pending_url = null // like the core: an explicit cancel forgets the link
      onEvent({ type: 'error', error: { code, message, detail } })
      return null
    }
    const cancelled = () => this.cancelled.has(job)
    onEvent({ type: 'stage', stage: 'downloading' })
    await this.checkpoint('stt:downloading')
    await this.sleep(350)
    if (cancelled()) return fail('cancelled', 'Cancelled')
    const rule = DOWNLOAD_RULES.find(([re]) => re.test(url))
    // refusals that need only the page, not the audio
    if (rule && ['download_live', 'download_login', 'download_cookies', 'download_drm', 'download_not_found', 'download_network', 'download_no_audio', 'download_failed'].includes(rule[1])) {
      if (rule[1] === 'download_failed') {
        onEvent({ type: 'downloader_fallback', from: 'bundled', to: 'system', message: 'Bundled downloader failed, retried with system yt-dlp' })
        await this.sleep(300)
      }
      return fail(rule[1], rule[2], `yt-dlp: ${rule[1]} (mock)`)
    }
    if (/\/fallback/.test(url)) {
      onEvent({ type: 'downloader_fallback', from: 'bundled', to: 'system', message: 'Bundled downloader failed, retried with system yt-dlp' })
      await this.sleep(300)
    }
    const title = titleFor(url)
    onEvent({ type: 'download_info', title, duration_s: 3725, playlist: /\/playlist/.test(url) })
    if (this.settings.rate_limit_sleep) {
      const secs = 7
      onEvent({ type: 'download_waiting', secs })
      await this.checkpoint('stt:waiting')
      for (let i = 0; i < secs * 4; i++) {
        if (cancelled()) return fail('cancelled', 'Cancelled')
        await this.sleep(250)
      }
    }
    const totalBytes = 38_400_000
    for (let i = 1; i <= 10; i++) {
      if (cancelled()) return fail('cancelled', 'Cancelled')
      await this.sleep(400)
      if (i === 4 && rule?.[1] === 'disk_space') return fail('disk_space', rule[2], 'No space left on device (mock)')
      onEvent({ type: 'download_progress', downloaded_bytes: (totalBytes * i) / 10, total_bytes: totalBytes, speed_bps: 2_400_000, eta_s: Math.round(((10 - i) * 0.4 * 6) / 1) })
      if (i === 5) await this.checkpoint('stt:download')
    }
    const id = `dl-${hash(url).padStart(16, '0')}`
    const path = `/mock/downloads/${id}/audio.m4a`
    this.workspace.pending_url = null
    onEvent({ type: 'downloaded', download_id: id, media_path: path, title, bytes: totalBytes })
    return { id, path, title }
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
    return VOICES.map(([id, label, language]) => ({ id, label, language, preview_path: `/mock/previews/${id}.mp3` }))
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
      this.durations.set(path, Math.round(totalMs / segCount))
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
  async exportDownload(_id: string, format: AudioExportFormat | null, dest: string) {
    return format ? ensureExt(dest, format) : dest
  }
  async downloadsInfo(): Promise<DownloadsInfoDto> {
    if (!this.settings.allow_downloads) throw new BackendError({ code: 'download_disabled', message: DOWNLOADS_OFF, detail: null })
    return {
      tools: [
        { kind: 'bundled', path: '/Applications/Parlo.app/Contents/Resources/yt-dlp/yt-dlp', version: '2026.08.19', impersonate: true, error: null },
        ...(this.settings.downloader_path === '/nonexistent'
          ? []
          : [{ kind: 'system', path: this.settings.downloader_path || '/opt/homebrew/bin/yt-dlp', version: '2026.10.02', impersonate: false, error: null }]),
      ],
      curl: '/usr/bin/curl',
      deno: '/opt/homebrew/bin/deno',
      browsers: [
        { id: 'chrome', label: 'Google Chrome' },
        { id: 'firefox', label: 'Firefox' },
        { id: 'safari', label: 'Safari' },
      ],
    }
  }

  // ------------------------------------------------------------------------------------------------------ workspace etc.
  async workspaceGet() {
    return structuredClone(this.workspace)
  }
  async workspaceUpdate(patch: WorkspacePatch) {
    if (patch.clear_transcript || patch.transcript) this.workspace.transcript_parked = false
    if (patch.clear_transcript) this.workspace.transcript = null
    if (patch.transcript) this.workspace.transcript = patch.transcript
    if (patch.clear_tts) this.workspace.tts = null
    if (patch.tts) this.workspace.tts = patch.tts
  }
  private storage: Record<string, number> = { recordings: 412_000_000, outputs: 186_000_000, cache: 73_000_000, models: 1_354_000_000 }
  async storageUsage(): Promise<StorageEntryDto[]> {
    const label: Record<string, string> = { recordings: 'Recordings', outputs: 'Generated speech', cache: 'Cache', models: 'Models' }
    return Object.entries(this.storage).map(([kind, bytes]) => ({ kind, label: label[kind]!, bytes, path: `/mock/Parlo/${kind}` }))
  }
  async storageClear(kind: 'outputs' | 'cache') {
    this.storage[kind] = 0
    return this.storageUsage()
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
  async pickFile(kind: FilePickKind): Promise<string | null> {
    return kind === 'document' ? '/Users/me/Documents/meeting-notes.docx' : '/Users/me/Downloads/team-meeting.m4a'
  }
  async saveFile(defaultName: string, _extensions: string[]): Promise<string | null> {
    return `/Users/me/Desktop/${defaultName}`
  }
  private durations = new Map<string, number>()
  private blobs = new Map<string, string>()
  /** a silent WAV of the length the scripts announced for this path (10 s if unknown), so the player really plays */
  fileUrl(path: string) {
    const cached = this.blobs.get(path)
    if (cached) return cached
    const secs = Math.min(600, Math.max(1, (this.durations.get(path) ?? 10_000) / 1000))
    const rate = 8000
    const n = Math.round(secs * rate)
    const buf = new ArrayBuffer(44 + n)
    const v = new DataView(buf)
    const w = (o: number, s: string) => [...s].forEach((c, i) => v.setUint8(o + i, c.charCodeAt(0)))
    w(0, 'RIFF'); v.setUint32(4, 36 + n, true); w(8, 'WAVE'); w(12, 'fmt '); v.setUint32(16, 16, true)
    v.setUint16(20, 1, true); v.setUint16(22, 1, true); v.setUint32(24, rate, true); v.setUint32(28, rate, true)
    v.setUint16(32, 1, true); v.setUint16(34, 8, true); w(36, 'data'); v.setUint32(40, n, true)
    new Uint8Array(buf, 44).fill(128)
    const url = URL.createObjectURL(new Blob([buf], { type: 'audio/wav' }))
    this.blobs.set(path, url)
    return url
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

function hash(s: string) {
  let h = 5381
  for (let i = 0; i < s.length; i++) h = ((h << 5) + h + s.charCodeAt(i)) | 0
  return (h >>> 0).toString(16)
}
function ensureExt(dest: string, ext: string) {
  return dest.toLowerCase().endsWith(`.${ext}`) ? dest : `${dest}.${ext}`
}
