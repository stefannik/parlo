import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { backend } from '../backend'
import type { IpcError, SttEventDto, TranscriptDto, TranscriptExportFormat, WorkspaceTranscriptDto } from '../bindings'
import { baseName, countWords, formatClock, stripExt } from '../lib/format'
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

export type SttStage = 'preparing' | 'detecting_speech' | 'detecting_language' | 'loading_model' | 'transcribing'
export const STAGE_ORDER: SttStage[] = ['preparing', 'detecting_speech', 'detecting_language', 'loading_model', 'transcribing']
export const STAGE_LABEL: Record<SttStage, string> = {
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

  /** audio file the mini player plays: the recording's WAV, or the original file */
  const playablePath = computed(() => {
    const s = source.value
    if (!s) return null
    return s.path
  })

  function resetRun() {
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
        { source: src.recordingId ? { kind: 'recording', recording_id: src.recordingId } : { kind: 'path', path: src.path! }, language: lang },
        handle,
      )
    } catch (e) {
      handle({ type: 'error', error: asIpc(e) })
    }
  }

  const transcribeFile = (path: string, lang = language.value) => run({ name: baseName(path), path, recordingId: null }, lang)
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

  async function saveAudio(format: 'wav' | 'm4a' | 'mp3' = 'wav') {
    const id = source.value?.recordingId
    if (!id) return
    const dest = await backend().saveFile(`${stripExt(source.value!.name)}.${format}`, [format])
    if (!dest) return
    try {
      const written = await backend().exportRecording(id, format, dest)
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
    }
    try {
      await backend().workspaceUpdate(wsPatch({ transcript: dto }))
    } catch {
      /* non-fatal: workspace is a convenience */
    }
  }

  function restore(w: WorkspaceTranscriptDto) {
    resetRun()
    source.value = { name: w.source_name, path: null, recordingId: w.recording_id }
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
