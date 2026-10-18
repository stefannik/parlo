import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { backend } from '../backend'
import type { IpcError, SttEventDto, TranscriptDto, TranscriptExportFormat, WorkspaceTranscriptDto } from '../bindings'
import { baseName, countWords, formatClock, stripExt } from '../lib/format'
import { checkUrl, fallbackTitle } from '../lib/url'
import { wsPatch } from '../lib/workspace'
import { useAppStore } from './app'
import { useRecorderStore } from './recorder'
import { useToastStore } from './toast'

export type TranscribePhase = 'empty' | 'recording' | 'processing' | 'result' | 'error'

export interface Paragraph {
  index: number
  text: string
  startS: number
  endS: number
}

export type SttStage = 'downloading' | 'preparing' | 'detecting_speech' | 'detecting_language' | 'loading_model' | 'transcribing'
export const STAGE_ORDER: SttStage[] = ['downloading', 'preparing', 'detecting_speech', 'detecting_language', 'loading_model', 'transcribing']
export const STAGE_LABEL: Record<SttStage, string> = {
  downloading: 'Downloading audio',
  preparing: 'Preparing audio',
  detecting_speech: 'Finding speech',
  detecting_language: 'Detecting language',
  loading_model: 'Loading speech model',
  transcribing: 'Transcribing',
}

interface Source {
  name: string
  path: string | null
  recordingId: string | null
  /** the link this audio comes from (URL jobs) */
  url?: string | null
  /** directory id of the kept download (`dl-…`), once the audio is on disk */
  downloadId?: string | null
}

/** What the Downloading stage knows: title, bytes, speed, the rate-limit wait and a fallback notice. */
export interface DownloadState {
  title: string | null
  durationS: number | null
  playlist: boolean
  downloaded: number
  total: number | null
  speedBps: number | null
  etaS: number | null
  /** seconds left of the rate-limit sleep (counts down once a second while shown), `null` when not waiting */
  waitLeft: number | null
  /** e.g. "Bundled downloader failed, retried with system yt-dlp" */
  fallback: string | null
}

/**
 * Transcribe view state machine:
 *   empty ──(file / stop recording)──▶ processing ──▶ result
 *     │ ▲                                   │ └──▶ error ──(retry)──▶ processing
 *     └─┴─(recorder active)─▶ recording     └──(cancel)──▶ empty
 * `recording` is derived from the recorder store so that a window re-opened mid-recording shows it immediately.
 */
export const useTranscribeStore = defineStore('transcribe', () => {
  const app = useAppStore()
  const recorder = useRecorderStore()
  const toast = useToastStore()

  const stage = ref<'idle' | 'processing' | 'result' | 'error'>('idle')
  const source = ref<Source | null>(null)
  const language = ref('auto') // language to request: "auto" or a code
  const currentStage = ref<SttStage>('preparing')
  const stageFraction = ref(0)
  const detectedLanguage = ref<{ code: string; prob: number } | null>(null)
  const progress = ref<{ doneS: number; totalS: number; etaMs: number | null } | null>(null)
  const paragraphs = ref<Paragraph[]>([])
  const transcript = ref<TranscriptDto | null>(null)
  const text = ref('') // editable transcript
  const edited = ref(false)
  const exportedOrCopied = ref(false)
  const error = ref<IpcError | null>(null)
  const jobId = ref<number | null>(null)
  const cancelling = ref(false)
  const detailsOpen = ref(false)
  // --- "Transcribe from URL"
  /** the link field's text; survives an error or cancel so retrying is one click, refilled from the workspace on launch */
  const urlDraft = ref('')
  const urlTouched = ref(false)
  const download = ref<DownloadState | null>(null)
  let waitTimer: ReturnType<typeof setInterval> | null = null

  const phase = computed<TranscribePhase>(() => {
    if (recorder.active) return 'recording'
    if (stage.value === 'processing') return 'processing'
    if (stage.value === 'result') return 'result'
    if (stage.value === 'error') return 'error'
    return 'empty'
  })

  const overallFraction = computed(() => {
    if (currentStage.value === 'transcribing' && progress.value && progress.value.totalS > 0) return Math.min(1, progress.value.doneS / progress.value.totalS)
    return null
  })
  const words = computed(() => countWords(text.value))
  /** inline problem with the typed link (only after the user tried to start, or left the field) */
  const urlError = computed(() => (urlTouched.value ? checkUrl(urlDraft.value).problem : null))
  const downloadFraction = computed(() => {
    const d = download.value
    if (!d || d.waitLeft != null || !d.total) return null
    return Math.min(1, d.downloaded / d.total)
  })
  /** stage list shows "Downloading" only for URL jobs */
  const isUrlJob = computed(() => !!source.value?.url)

  /** audio file the mini player plays: the recording's WAV, or the original file */
  const playablePath = computed(() => {
    const s = source.value
    if (!s) return null
    return s.path
  })

  function stopWaitTimer() {
    if (waitTimer) clearInterval(waitTimer)
    waitTimer = null
  }
  /** "Waiting N s": the only timer here, alive just while the countdown is on screen */
  function startWait(secs: number) {
    stopWaitTimer()
    const end = Date.now() + secs * 1000
    if (download.value) download.value.waitLeft = Math.max(0, Math.ceil(secs))
    waitTimer = setInterval(() => {
      const left = Math.max(0, Math.ceil((end - Date.now()) / 1000))
      if (download.value) download.value.waitLeft = left
      if (left <= 0) stopWaitTimer()
    }, 250)
  }

  function resetRun() {
    stopWaitTimer()
    download.value = null
    currentStage.value = 'preparing'
    stageFraction.value = 0
    detectedLanguage.value = null
    progress.value = null
    paragraphs.value = []
    error.value = null
    detailsOpen.value = false
    cancelling.value = false
  }

  function handle(ev: SttEventDto) {
    switch (ev.type) {
      case 'stage':
        currentStage.value = ev.stage as SttStage
        stageFraction.value = 0
        if (ev.stage === 'downloading' && !download.value) {
          download.value = { title: null, durationS: null, playlist: false, downloaded: 0, total: null, speedBps: null, etaS: null, waitLeft: null, fallback: null }
        }
        break
      case 'download_info':
        if (download.value) Object.assign(download.value, { title: ev.title, durationS: ev.duration_s, playlist: ev.playlist })
        if (source.value) source.value = { ...source.value, name: ev.title } // job name = the video/episode title
        break
      case 'download_waiting':
        startWait(ev.secs)
        break
      case 'download_progress':
        stopWaitTimer()
        if (download.value) Object.assign(download.value, { waitLeft: null, downloaded: ev.downloaded_bytes, total: ev.total_bytes, speedBps: ev.speed_bps, etaS: ev.eta_s })
        break
      case 'downloader_fallback':
        if (download.value) download.value.fallback = ev.message
        break
      case 'downloaded':
        stopWaitTimer()
        if (download.value) download.value.waitLeft = null
        if (source.value) source.value = { ...source.value, name: ev.title, path: ev.media_path, downloadId: ev.download_id }
        urlDraft.value = '' // the link did its job; a failure before this point keeps it for a one-click retry
        break
      case 'stage_progress':
        stageFraction.value = ev.fraction
        break
      case 'language':
        detectedLanguage.value = { code: ev.code, prob: ev.prob }
        break
      case 'progress':
        progress.value = { doneS: ev.done_s, totalS: ev.total_s, etaMs: ev.eta_ms }
        break
      case 'paragraph':
        paragraphs.value = [...paragraphs.value.filter((p) => p.index !== ev.index), { index: ev.index, text: ev.text, startS: ev.start_s, endS: ev.end_s }].sort(
          (a, b) => a.index - b.index,
        )
        break
      case 'done':
        stopWaitTimer()
        if (source.value && ev.transcript.download_id) source.value = { ...source.value, path: ev.transcript.media_path ?? source.value.path, downloadId: ev.transcript.download_id, url: ev.transcript.source_url ?? source.value.url }
        transcript.value = ev.transcript
        text.value = ev.transcript.text
        edited.value = false
        exportedOrCopied.value = false
        stage.value = 'result'
        jobId.value = null
        void persist()
        break
      case 'error':
        jobId.value = null
        stopWaitTimer()
        if (download.value) download.value.waitLeft = null
        if (ev.error.code === 'cancelled') {
          stage.value = 'idle'
          source.value = null
        } else {
          error.value = ev.error
          stage.value = 'error'
        }
        break
    }
  }

  async function run(src: Source, lang: string) {
    // Never start while another transcription is running.
    if (stage.value === 'processing' || recorder.active) return
    app.tab = 'transcribe'
    source.value = src
    language.value = lang
    resetRun()
    transcript.value = null
    text.value = ''
    stage.value = 'processing'
    try {
      jobId.value = await backend().transcribe(
        {
          source: src.recordingId
            ? { kind: 'recording', recording_id: src.recordingId }
            : src.path
              ? { kind: 'path', path: src.path }
              : { kind: 'url', url: src.url! }, // the audio isn't on disk yet: the core downloads it first
          language: lang,
        },
        handle,
      )
    } catch (e) {
      handle({ type: 'error', error: asIpc(e) })
    }
  }

  const transcribeFile = (path: string, lang = language.value) => run({ name: baseName(path), path, recordingId: null }, lang)

  /** Start "Transcribe from URL" with `link` (default: the field's text). Invalid links never leave the UI. */
  async function transcribeUrl(link = urlDraft.value, lang = language.value): Promise<boolean> {
    if (stage.value === 'processing' || recorder.active) return false
    if (app.settings && !app.settings.allow_downloads) {
      toast.error('Downloads from links are turned off. Turn them on in Settings → Downloads.')
      return false
    }
    urlTouched.value = true
    const c = checkUrl(link)
    if (!c.url) return false
    urlDraft.value = c.url
    urlTouched.value = false
    await run({ name: fallbackTitle(c.url), path: null, recordingId: null, url: c.url, downloadId: null }, lang)
    return true
  }
  const transcribeRecording = (id: string, wavPath: string, name: string, lang = language.value) => run({ name, path: wavPath, recordingId: id }, lang)

  /** ⌘R / the Record card: asks for mic permission first when it hasn't been decided. */
  async function startRecording() {
    if (recorder.active || stage.value === 'processing') return
    if (recorder.permission === 'not_determined') {
      const p = await recorder.requestPermission()
      if (p !== 'granted') return
    }
    app.tab = 'transcribe'
    await recorder.start(app.settings?.input_device ?? null)
  }

  async function stopAndTranscribe() {
    if (!recorder.active) return
    try {
      const info = await recorder.stop()
      const when = new Date(info.started_at)
      const name = `Recording ${when.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })} ${when.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })}`
      await transcribeRecording(info.id, info.wav_path, name)
    } catch (e) {
      toast.error(`Couldn't finish the recording: ${asIpc(e).message}`)
    }
  }

  async function discardRecording() {
    if (!recorder.active) return
    const ok = await app.ask({
      title: 'Discard this recording?',
      body: `The ${formatClock(recorder.elapsedMs)} recorded so far will be deleted. This can’t be undone.`,
      confirmLabel: 'Discard',
      danger: true,
    })
    if (ok) await recorder.discard()
  }

  /** ⌘R toggles: start when idle, stop & transcribe while recording. */
  async function toggleRecording() {
    if (recorder.active) await stopAndTranscribe()
    else await startRecording()
  }

  /** Resume an interrupted job found at launch (the core continues from its per-chunk checkpoints). */
  async function resumeJob(id: string, name: string) {
    if (stage.value === 'processing' || recorder.active) return
    app.tab = 'transcribe'
    source.value = { name, path: null, recordingId: null }
    resetRun()
    transcript.value = null
    text.value = ''
    stage.value = 'processing'
    try {
      jobId.value = await backend().sttResume(id, handle)
    } catch (e) {
      handle({ type: 'error', error: asIpc(e) })
    }
  }

  async function retry() {
    if (source.value) await run(source.value, language.value)
  }

  /** Change the language of an existing result and re-run (asks first if the transcript was edited). */
  async function retranscribe(lang: string) {
    if (!source.value) return
    if (edited.value) {
      const ok = await app.ask({
        title: 'Replace your edits?',
        body: 'Transcribing again in another language will replace the text you edited.',
        confirmLabel: 'Transcribe again',
        danger: true,
      })
      if (!ok) return
    }
    await run(source.value, lang)
  }

  async function cancel() {
    if (jobId.value == null || cancelling.value) return
    cancelling.value = true
    await backend().jobCancel(jobId.value)
  }

  function edit(value: string) {
    text.value = value
    edited.value = true
    exportedOrCopied.value = false
    schedulePersist()
  }

  async function copy() {
    await backend().copyText(text.value)
    exportedOrCopied.value = true
    toast.success('Transcript copied')
    void persist()
  }

  async function exportAs(format: TranscriptExportFormat) {
    const t = transcript.value
    if (!t) return
    const name = `${stripExt(t.source_name)}.${format}`
    const dest = await backend().saveFile(name, [format])
    if (!dest) return
    try {
      const written = await backend().exportTranscript(text.value, format, dest, { source_name: t.source_name, language: t.language, duration_s: t.duration_s })
      exportedOrCopied.value = true
      toast.success(`Saved ${baseName(written)}`, { action: { label: 'Show in Finder', run: () => void backend().reveal(written) } })
      void persist()
    } catch (e) {
      toast.error(`Couldn't export: ${asIpc(e).message}`)
    }
  }

  /** `original` (downloaded audio only) copies the file as it came from the site; the others convert. */
  async function saveAudio(format: 'original' | 'wav' | 'm4a' | 'mp3' = 'wav') {
    const rid = source.value?.recordingId
    const did = source.value?.downloadId
    if (!rid && !did) return
    const ext = format === 'original' ? (source.value?.path?.split('.').pop() ?? 'm4a') : format
    const dest = await backend().saveFile(`${stripExt(source.value!.name)}.${ext}`, [ext])
    if (!dest) return
    try {
      const written = rid ? await backend().exportRecording(rid, format as 'wav' | 'm4a' | 'mp3', dest) : await backend().exportDownload(did!, format === 'original' ? null : format, dest)
      toast.success(`Saved ${baseName(written)}`, { action: { label: 'Show in Finder', run: () => void backend().reveal(written) } })
    } catch (e) {
      toast.error(`Couldn't save audio: ${asIpc(e).message}`)
    }
  }

  /** "New": asks only if the transcript was edited and never copied/exported. */
  async function startNew(): Promise<boolean> {
    if (stage.value === 'result' && edited.value && !exportedOrCopied.value) {
      const ok = await app.ask({
        title: 'Discard this transcript?',
        body: 'You edited the transcript and haven’t copied or exported it. Starting a new one will discard it.',
        confirmLabel: 'Discard',
        danger: true,
      })
      if (!ok) return false
    }
    stage.value = 'idle'
    source.value = null
    transcript.value = null
    text.value = ''
    edited.value = false
    resetRun()
    urlDraft.value = ''
    urlTouched.value = false
    await backend().workspaceUpdate(wsPatch({ clear_transcript: true }))
    return true
  }

  // --- workspace persistence (so the result survives closing the window / quitting) ---
  let persistTimer: ReturnType<typeof setTimeout> | null = null
  function schedulePersist() {
    if (persistTimer) clearTimeout(persistTimer)
    persistTimer = setTimeout(() => void persist(), 800)
  }
  async function persist() {
    if (persistTimer) clearTimeout(persistTimer)
    persistTimer = null
    const t = transcript.value
    if (!t || stage.value !== 'result') return
    const dto: WorkspaceTranscriptDto = {
      text: text.value,
      source_name: t.source_name,
      language: t.language,
      duration_s: t.duration_s,
      words: countWords(text.value),
      edited: edited.value,
      exported_or_copied: exportedOrCopied.value,
      recording_id: t.recording_id,
      source_url: t.source_url,
      download_id: t.download_id,
      media_path: null, // the core owns this one
    }
    try {
      await backend().workspaceUpdate(wsPatch({ transcript: dto }))
    } catch {
      /* non-fatal: workspace is a convenience */
    }
  }

  function restore(w: WorkspaceTranscriptDto) {
    resetRun()
    source.value = { name: w.source_name, path: w.media_path, recordingId: w.recording_id, url: w.source_url, downloadId: w.download_id }
    transcript.value = {
      text: w.text,
      language: w.language,
      language_prob: null,
      duration_s: w.duration_s,
      speech_s: w.duration_s,
      words: w.words,
      processing_s: 0,
      source_name: w.source_name,
      recording_id: w.recording_id,
      source_url: w.source_url,
      download_id: w.download_id,
      media_path: w.media_path,
    }
    text.value = w.text
    edited.value = w.edited
    exportedOrCopied.value = w.exported_or_copied
    stage.value = 'result'
  }

  return {
    stage,
    phase,
    source,
    language,
    currentStage,
    stageFraction,
    detectedLanguage,
    progress,
    overallFraction,
    paragraphs,
    transcript,
    text,
    words,
    edited,
    exportedOrCopied,
    error,
    detailsOpen,
    cancelling,
    jobId,
    playablePath,
    urlDraft,
    urlTouched,
    urlError,
    download,
    downloadFraction,
    isUrlJob,
    transcribeUrl,
    transcribeFile,
    transcribeRecording,
    retry,
    startRecording,
    stopAndTranscribe,
    discardRecording,
    toggleRecording,
    resumeJob,
    retranscribe,
    cancel,
    edit,
    copy,
    exportAs,
    saveAudio,
    startNew,
    restore,
    persist,
  }
})

export function asIpc(e: unknown): IpcError {
  if (e && typeof e === 'object' && 'code' in e && 'message' in e) {
    const o = e as { code: string; message: string; detail?: string | null }
    return { code: o.code, message: o.message, detail: o.detail ?? null }
  }
  return { code: 'other', message: e instanceof Error ? e.message : String(e), detail: null }
}
