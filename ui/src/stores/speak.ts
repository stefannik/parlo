import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { backend } from '../backend'
import type { AudioExportFormat, ImportResultDto, IpcError, TtsEventDto, VoiceDto, WorkspaceTtsDto } from '../bindings'
import { baseName, countWords, listeningMinutes, stripExt } from '../lib/format'
import { wsPatch } from '../lib/workspace'
import { useAppStore } from './app'
import { asIpc } from './transcribe'
import { useToastStore } from './toast'

export type SpeakPhase = 'compose' | 'generating' | 'result' | 'error'

export interface Segment {
  index: number
  path: string
  durationMs: number
}

/**
 * Speak view state machine:
 *   compose ──generate──▶ generating ──done──▶ result ──(edit text/voice)──▶ result (out of date) ──regenerate──▶ generating
 *                            │  └──error──▶ error ──retry──▶ generating
 *                            └──cancel──▶ compose
 * The player becomes active on the first `segment_ready` (listen while it generates).
 */
export const useSpeakStore = defineStore('speak', () => {
  const app = useAppStore()
  const toast = useToastStore()

  const text = ref('')
  const imported = ref<ImportResultDto | null>(null)
  const voices = ref<VoiceDto[]>([])
  const voice = ref('george')
  const speed = ref(1)
  const quality = ref('standard')
  const phase = ref<SpeakPhase>('compose')
  const stageLabel = ref('')
  const progress = ref<{ doneChars: number; totalChars: number; etaMs: number | null } | null>(null)
  const segments = ref<Segment[]>([])
  const outputId = ref<string | null>(null)
  const durationMs = ref<number | null>(null)
  const cacheHit = ref(false)
  const error = ref<IpcError | null>(null)
  const jobId = ref<number | null>(null)
  const cancelling = ref(false)
  const importing = ref(false)
  /** what the current output was generated from */
  const generatedFrom = ref<{ text: string; voice: string; importId: string | null } | null>(null)
  const sourceName = ref<string | null>(null)

  const effectiveText = computed(() => (imported.value?.large && imported.value.text == null ? imported.value.preview : text.value))
  /** large documents are read in the core; the editor shows a preview and is read-only */
  const readOnly = computed(() => !!imported.value && imported.value.text == null)
  const words = computed(() => (readOnly.value ? imported.value!.words : countWords(text.value)))
  const chars = computed(() => (readOnly.value ? imported.value!.chars : text.value.length))
  const minutes1x = computed(() => listeningMinutes(words.value, 1))
  const minutesAtSpeed = computed(() => listeningMinutes(words.value, speed.value))
  const canGenerate = computed(() => phase.value !== 'generating' && !importing.value && (readOnly.value || text.value.trim().length > 0))
  const outOfDate = computed(() => {
    const g = generatedFrom.value
    if (!g || phase.value !== 'result') return false
    return g.voice !== voice.value || (g.importId ?? '') !== (imported.value?.import_id ?? '') || (!readOnly.value && g.text !== text.value)
  })
  const hasOutput = computed(() => segments.value.length > 0)

  async function loadVoices() {
    voices.value = await backend().voices()
    if (!voices.value.some((v) => v.id === voice.value) && voices.value.length) voice.value = voices.value[0]!.id
  }

  function applyDefaults() {
    const s = app.settings
    if (!s) return
    speed.value = s.default_speed
    quality.value = s.tts_quality
    if (voices.value.some((v) => v.id === s.default_voice)) voice.value = s.default_voice
  }

  function setText(v: string) {
    text.value = v
    // typing over an imported document detaches the source chip
    if (imported.value && !readOnly.value && v !== imported.value.text) imported.value = null
    schedulePersist()
  }

  function clear() {
    text.value = ''
    imported.value = null
    schedulePersist()
  }

  async function importFile(path: string) {
    app.tab = 'speak'
    importing.value = true
    try {
      const r = await backend().importText(path)
      imported.value = r
      text.value = r.text ?? r.preview
      sourceName.value = r.file_name
      if (r.warnings.length) toast.info(r.warnings[0]!)
      schedulePersist()
    } catch (e) {
      toast.error(`Couldn't open “${baseName(path)}”: ${asIpc(e).message}`)
    } finally {
      importing.value = false
    }
  }

  function handle(ev: TtsEventDto) {
    switch (ev.type) {
      case 'stage':
        stageLabel.value = ev.stage
        break
      case 'progress':
        progress.value = { doneChars: ev.done_chars, totalChars: ev.total_chars, etaMs: ev.eta_ms }
        break
      case 'segment_ready':
        segments.value = [...segments.value.filter((s) => s.index !== ev.index), { index: ev.index, path: ev.path, durationMs: ev.duration_ms }].sort((a, b) => a.index - b.index)
        break
      case 'done':
        outputId.value = ev.output_id
        durationMs.value = ev.duration_ms
        cacheHit.value = ev.cache_hit
        phase.value = 'result'
        jobId.value = null
        void persist()
        break
      case 'error':
        jobId.value = null
        if (ev.error.code === 'cancelled') {
          phase.value = segments.value.length ? 'result' : 'compose'
          if (!outputId.value) segments.value = []
        } else {
          error.value = ev.error
          phase.value = 'error'
        }
        break
    }
  }

  async function generate() {
    if (!canGenerate.value) return
    app.tab = 'speak'
    error.value = null
    segments.value = []
    outputId.value = null
    durationMs.value = null
    progress.value = null
    cancelling.value = false
    stageLabel.value = 'preparing'
    phase.value = 'generating'
    const imp = readOnly.value ? imported.value : null
    generatedFrom.value = { text: text.value, voice: voice.value, importId: imported.value?.import_id ?? null }
    try {
      jobId.value = await backend().tts({ source: imp ? { kind: 'import', import_id: imp.import_id } : { kind: 'text', text: text.value }, voice: voice.value, quality: quality.value }, handle)
    } catch (e) {
      handle({ type: 'error', error: asIpc(e) })
    }
  }

  async function resumeJob(id: string) {
    app.tab = 'speak'
    error.value = null
    segments.value = []
    outputId.value = null
    progress.value = null
    cancelling.value = false
    stageLabel.value = 'preparing'
    phase.value = 'generating'
    try {
      jobId.value = await backend().ttsResume(id, handle)
    } catch (e) {
      handle({ type: 'error', error: asIpc(e) })
    }
  }

  async function cancel() {
    if (jobId.value == null || cancelling.value) return
    cancelling.value = true
    await backend().jobCancel(jobId.value)
  }

  async function exportAs(format: AudioExportFormat) {
    if (!outputId.value) return
    const base = stripExt(sourceName.value ?? 'Parlo speech')
    const dest = await backend().saveFile(`${base}.${format}`, [format])
    if (!dest) return
    try {
      const written = await backend().exportTts(outputId.value, format, dest)
      toast.success(`Saved ${baseName(written)}`, { action: { label: 'Show in Finder', run: () => void backend().reveal(written) } })
    } catch (e) {
      toast.error(`Couldn't export: ${asIpc(e).message}`)
    }
  }

  async function reveal() {
    const first = segments.value[0]?.path
    if (first) await backend().reveal(first)
  }

  // --- workspace persistence ---
  let persistTimer: ReturnType<typeof setTimeout> | null = null
  function schedulePersist() {
    if (persistTimer) clearTimeout(persistTimer)
    persistTimer = setTimeout(() => void persist(), 800)
  }
  async function persist() {
    if (persistTimer) clearTimeout(persistTimer)
    persistTimer = null
    const dto: WorkspaceTtsDto = {
      text: text.value,
      voice: voice.value,
      output_id: phase.value === 'result' ? outputId.value : null,
      source_name: sourceName.value,
      segments: phase.value === 'result' ? segments.value.map((s) => s.path) : [],
      duration_ms: phase.value === 'result' ? durationMs.value : null,
    }
    try {
      await backend().workspaceUpdate(text.value || dto.output_id ? wsPatch({ tts: dto }) : wsPatch({ clear_tts: true }))
    } catch {
      /* non-fatal */
    }
  }

  function restore(w: WorkspaceTtsDto) {
    text.value = w.text
    if (voices.value.some((v) => v.id === w.voice) || !voices.value.length) voice.value = w.voice
    sourceName.value = w.source_name
    if (w.output_id && w.segments.length) {
      outputId.value = w.output_id
      durationMs.value = w.duration_ms
      const per = w.duration_ms ? w.duration_ms / w.segments.length : 0
      segments.value = w.segments.map((path, index) => ({ index, path, durationMs: per }))
      generatedFrom.value = { text: w.text, voice: w.voice, importId: null }
      phase.value = 'result'
    }
  }

  return {
    text,
    imported,
    voices,
    voice,
    speed,
    quality,
    phase,
    stageLabel,
    progress,
    segments,
    outputId,
    durationMs,
    cacheHit,
    error,
    jobId,
    cancelling,
    importing,
    sourceName,
    effectiveText,
    readOnly,
    words,
    chars,
    minutes1x,
    minutesAtSpeed,
    canGenerate,
    outOfDate,
    hasOutput,
    loadVoices,
    applyDefaults,
    setText,
    clear,
    importFile,
    generate,
    resumeJob,
    cancel,
    exportAs,
    reveal,
    persist,
    restore,
  }
})
