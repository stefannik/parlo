<script setup lang="ts">
import { computed, ref } from 'vue'
import { backend } from '../backend'
import AudioPlayer from '../components/AudioPlayer.vue'
import Icon from '../components/Icon.vue'
import LanguageSelect from '../components/LanguageSelect.vue'
import LevelMeter from '../components/LevelMeter.vue'
import MenuButton from '../components/MenuButton.vue'
import { formatClock, formatDurationWords, formatEta } from '../lib/format'
import { languageName } from '../lib/languages'
import { useAppStore } from '../stores/app'
import { useRecorderStore } from '../stores/recorder'
import { STAGE_LABEL, STAGE_ORDER, useTranscribeStore, type SttStage } from '../stores/transcribe'

const app = useAppStore()
const rec = useRecorderStore()
const tr = useTranscribeStore()

const speed = ref(1)
const showLang = ref(false)

const visibleStages = computed<SttStage[]>(() => STAGE_ORDER.filter((s) => s !== 'detecting_language' || tr.language === 'auto'))
const stageIndex = computed(() => visibleStages.value.indexOf(tr.currentStage))
const subtitle = computed(() => {
  const parts: string[] = []
  if (tr.progress) parts.push(`${formatDurationWords(tr.progress.totalS)} of audio`)
  if (tr.detectedLanguage) parts.push(languageName(tr.detectedLanguage.code))
  return parts.join(' · ')
})
const pct = computed(() => (tr.overallFraction == null ? null : Math.round(tr.overallFraction * 100)))

const micHint = computed(() => {
  switch (rec.permission) {
    case 'denied':
    case 'restricted':
      return 'Microphone access is off'
    case 'not_determined':
      return 'Parlo will ask for microphone access'
    default:
      return rec.devices.find((d) => d.id === app.settings?.input_device)?.name ?? rec.devices.find((d) => d.is_default)?.name ?? 'System default input'
  }
})

async function chooseFile() {
  const p = await backend().pickFile('media')
  if (p) await tr.transcribeFile(p)
}
function openPrivacy() {
  void backend().openPrivacySettings('microphone')
}
function onLangChange(code: string) {
  showLang.value = false
  void tr.retranscribe(code)
}
function onEdit(e: Event) {
  tr.edit((e.target as HTMLTextAreaElement).value)
}
const exportItems = [
  { id: 'txt', label: 'Text (.txt)' },
  { id: 'md', label: 'Markdown (.md)' },
]
const audioItems = [
  { id: 'wav', label: 'WAV' },
  { id: 'm4a', label: 'M4A (AAC)' },
  { id: 'mp3', label: 'MP3' },
]
defineExpose({ chooseFile })
</script>

<template>
  <section class="view" aria-label="Transcribe" data-testid="transcribe-view" :data-phase="tr.phase">
    <!-- 1 · empty -->
    <div v-if="tr.phase === 'empty'" class="empty">
      <div class="cards">
        <div class="card pad choice">
          <button type="button" class="mic-btn" :disabled="rec.starting" aria-label="Start recording" data-testid="record-button" @click="tr.startRecording()">
            <Icon name="mic" :size="34" />
          </button>
          <h2>Record</h2>
          <p class="muted small" data-testid="mic-hint">{{ micHint }}</p>
          <div v-if="rec.permission === 'denied' || rec.permission === 'restricted'" class="inline-warn" role="alert">
            <Icon name="warn" :size="16" />
            <span>Parlo can’t use the microphone. Turn it on in System Settings → Privacy &amp; Security → Microphone.</span>
            <button type="button" class="btn" @click="openPrivacy">Open System Settings</button>
          </div>
          <p v-else-if="rec.lastError" class="inline-warn" role="alert"><Icon name="warn" :size="16" /> {{ rec.lastError }}</p>
          <span class="kbd small muted">⌘R</span>
        </div>
        <div class="card pad choice drop" :class="{ hover: app.dropHover }" data-testid="drop-zone">
          <div class="drop-icon"><Icon name="upload" :size="30" /></div>
          <h2>Transcribe a file</h2>
          <p class="muted small">Drop an audio or video file here</p>
          <button type="button" class="btn btn-primary" data-testid="choose-file" @click="chooseFile">Choose file…</button>
          <p class="muted tiny">MP3 · M4A · WAV · FLAC · OGG/Opus · AAC · AIFF · MP4 · MOV · MKV · WebM</p>
        </div>
      </div>
      <div class="lang-row">
        <LanguageSelect v-model="tr.language" label="Language" />
      </div>
    </div>

    <!-- 2 · recording -->
    <div v-else-if="tr.phase === 'recording'" class="recording">
      <div class="rec-head">
        <span class="rec-pill" :class="{ paused: rec.state === 'paused' }"><span class="dot" /> {{ rec.state === 'paused' ? 'Paused' : 'Recording' }}</span>
      </div>
      <div class="timer tabular" data-testid="rec-timer" role="timer" aria-live="off">{{ formatClock(rec.elapsedMs) }}</div>
      <LevelMeter :rms="rec.rms" :peak="rec.peak" :active="rec.state === 'recording'" />
      <p class="muted small device">
        <Icon name="mic" :size="14" /> {{ rec.device || 'Microphone' }}
        <span class="sep">·</span>
        <span class="saved" data-testid="saved-indicator"><Icon name="check" :size="14" /> Saved continuously</span>
      </p>
      <div class="banners" aria-live="polite">
        <div v-for="b in rec.banners" :key="b.id" class="banner" :class="b.tone" role="status" data-testid="rec-banner">
          <Icon :name="b.tone === 'info' ? 'info' : 'warn'" :size="16" />
          <span>{{ b.text }}</span>
          <button type="button" class="btn btn-ghost btn-icon" aria-label="Dismiss" @click="rec.dismissBanner(b.id)"><Icon name="x" :size="14" /></button>
        </div>
      </div>
      <div class="rec-actions">
        <button type="button" class="btn btn-lg" data-testid="pause-button" @click="rec.togglePause()">
          <Icon :name="rec.state === 'paused' ? 'mic' : 'pause'" :size="16" /> {{ rec.state === 'paused' ? 'Resume' : 'Pause' }}
        </button>
        <button type="button" class="btn btn-lg btn-primary" data-testid="stop-button" @click="tr.stopAndTranscribe()"><Icon name="stop" :size="16" /> Stop &amp; Transcribe</button>
        <button type="button" class="btn btn-lg btn-ghost btn-danger" data-testid="discard-button" @click="tr.discardRecording()"><Icon name="trash" :size="16" /> Discard</button>
      </div>
    </div>

    <!-- 3 · processing -->
    <div v-else-if="tr.phase === 'processing'" class="processing">
      <header class="proc-head">
        <div>
          <h2 class="selectable" data-testid="source-name">{{ tr.source?.name }}</h2>
          <p v-if="subtitle" class="muted small" data-testid="proc-subtitle">{{ subtitle }}</p>
        </div>
        <button type="button" class="btn" :disabled="tr.cancelling" data-testid="cancel-button" @click="tr.cancel()">{{ tr.cancelling ? 'Cancelling…' : 'Cancel' }}</button>
      </header>
      <ol class="stages" aria-label="Progress" aria-live="polite">
        <li v-for="(s, i) in visibleStages" :key="s" :class="{ done: i < stageIndex, current: i === stageIndex }" :data-stage="s">
          <span class="mark"><Icon v-if="i < stageIndex" name="check" :size="12" /></span>
          {{ STAGE_LABEL[s] }}<template v-if="i === stageIndex && s === 'transcribing' && pct != null"> · {{ pct }}%</template>
        </li>
      </ol>
      <div class="bar-row">
        <div class="bar" role="progressbar" :aria-valuenow="pct ?? undefined" aria-valuemin="0" aria-valuemax="100" :aria-label="STAGE_LABEL[tr.currentStage]" :class="{ indeterminate: pct == null && tr.stageFraction === 0 }">
          <div class="fill" :style="{ width: `${Math.round((pct != null ? (tr.overallFraction ?? 0) : tr.stageFraction) * 100)}%` }" />
        </div>
        <span class="muted small tabular eta" data-testid="eta">{{ pct != null ? `${pct}% · ${formatEta(tr.progress?.etaMs)}` : tr.currentStage === 'loading_model' ? 'Loading speech model…' : '' }}</span>
      </div>
      <div v-if="tr.paragraphs.length" class="partial selectable" aria-label="Transcript so far" data-testid="partial">
        <p v-for="p in tr.paragraphs" :key="p.index">{{ p.text }}</p>
      </div>
    </div>

    <!-- 4 · result -->
    <div v-else-if="tr.phase === 'result' && tr.transcript" class="result">
      <header class="res-head">
        <div class="titles">
          <h2 class="selectable" data-testid="source-name">{{ tr.transcript.source_name }}</h2>
          <p class="muted small meta">
            <span>{{ formatDurationWords(tr.transcript.duration_s) }}</span>
            <span class="sep">·</span>
            <span data-testid="word-count">{{ tr.words.toLocaleString() }} words</span>
            <span class="sep">·</span>
            <button type="button" class="chip chip-accent lang-chip" title="Change language and transcribe again" data-testid="language-badge" @click="showLang = !showLang">
              {{ languageName(tr.transcript.language) }} <Icon name="chevron" :size="12" />
            </button>
            <LanguageSelect v-if="showLang" :model-value="tr.transcript.language ?? 'auto'" compact label="Transcribe again as" @update:model-value="onLangChange" />
          </p>
        </div>
        <div class="actions">
          <button type="button" class="btn" data-testid="copy-button" @click="tr.copy()"><Icon name="copy" :size="16" /> Copy</button>
          <MenuButton label="Export" :items="exportItems" icon="download" testid="export-button" :signal="app.exportSignal" @select="(id) => tr.exportAs(id as 'txt' | 'md')" />
          <MenuButton v-if="tr.source?.recordingId" label="Save audio" :items="audioItems" testid="save-audio-button" @select="(id) => tr.saveAudio(id as 'wav' | 'm4a' | 'mp3')" />
          <button type="button" class="btn btn-primary" data-testid="new-button" @click="tr.startNew()"><Icon name="plus" :size="16" /> New</button>
        </div>
      </header>
      <textarea class="editor" :value="tr.text" spellcheck="true" aria-label="Transcript" data-testid="transcript-editor" @input="onEdit" />
      <AudioPlayer v-if="tr.playablePath" :segments="[{ path: tr.playablePath, durationMs: tr.transcript.duration_s * 1000 }]" :total-ms="tr.transcript.duration_s * 1000" v-model:speed="speed" label="Source audio" />
    </div>

    <!-- 5 · error -->
    <div v-else-if="tr.phase === 'error' && tr.error" class="error">
      <div class="err-card card pad" role="alert" data-testid="error-card">
        <div class="err-icon"><Icon name="warn" :size="26" /></div>
        <h2>That didn’t work</h2>
        <p class="selectable">{{ tr.error.message }}</p>
        <p v-if="tr.source" class="muted small">Your file “{{ tr.source.name }}” hasn’t been changed.</p>
        <div class="err-actions">
          <button type="button" class="btn btn-primary" data-testid="retry-button" @click="tr.retry()"><Icon name="refresh" :size="16" /> Retry</button>
          <button type="button" class="btn" @click="tr.detailsOpen = !tr.detailsOpen">{{ tr.detailsOpen ? 'Hide details' : 'Show details' }}</button>
          <button type="button" class="btn btn-ghost" @click="tr.startNew()">Start over</button>
        </div>
        <div v-if="tr.detailsOpen" class="details">
          <pre class="selectable mono small" data-testid="error-details">{{ tr.error.detail ?? tr.error.code }}</pre>
          <button type="button" class="btn" @click="backend().copyText(`${tr.error!.code}: ${tr.error!.message}\n${tr.error!.detail ?? ''}`)"><Icon name="copy" :size="14" /> Copy details</button>
        </div>
      </div>
    </div>
  </section>
</template>

<style scoped>
.view { height: 100%; display: flex; flex-direction: column; padding: 0 var(--s5) var(--s6); min-height: 0; }
h2 { font-size: 15px; font-weight: 600; }
.pad { padding: var(--s5); }
.tiny { font-size: 11px; }

/* empty */
.empty { flex: 1; display: flex; flex-direction: column; justify-content: center; gap: var(--s5); min-height: 0; }
.cards { display: grid; grid-template-columns: 1fr 1fr; gap: var(--s5); }
.choice { display: flex; flex-direction: column; align-items: center; justify-content: center; text-align: center; gap: var(--s2); min-height: 280px; }
.mic-btn {
  width: 84px; height: 84px; border-radius: 50%; border: 0; background: var(--accent); color: var(--on-accent);
  display: grid; place-items: center; margin-bottom: var(--s3); box-shadow: 0 6px 18px var(--accent-soft);
  transition: background var(--dur) var(--ease), transform var(--dur) var(--ease);
}
.mic-btn:hover { background: var(--accent-hover); transform: scale(1.03); }
.mic-btn:active { transform: scale(0.98); }
.mic-btn[disabled] { opacity: 0.5; }
.drop { border-style: dashed; border-width: 2px; border-color: var(--border-strong); box-shadow: none; transition: background var(--dur) var(--ease), border-color var(--dur) var(--ease); }
.drop.hover { background: var(--accent-soft); border-color: var(--accent); }
.drop-icon { width: 84px; height: 84px; border-radius: 50%; display: grid; place-items: center; background: var(--surface-2); color: var(--text-2); margin-bottom: var(--s3); }
.drop .btn { margin-top: var(--s2); }
.kbd { margin-top: var(--s1); }
.inline-warn { display: flex; align-items: center; flex-wrap: wrap; justify-content: center; gap: var(--s2); padding: 8px 12px; border-radius: var(--r-sm); background: var(--warning-soft); color: var(--text); font-size: 12px; max-width: 360px; }
.inline-warn > svg { color: var(--warning); flex: none; }
.lang-row { display: flex; justify-content: center; }

/* recording */
.recording { flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: var(--s4); }
.rec-pill { display: inline-flex; align-items: center; gap: 8px; padding: 3px 12px; border-radius: 999px; background: var(--danger-soft); color: var(--danger); font-weight: 600; font-size: 12px; text-transform: uppercase; letter-spacing: 0.06em; }
.rec-pill .dot { width: 8px; height: 8px; border-radius: 50%; background: var(--danger); }
.rec-pill.paused { background: var(--surface-2); color: var(--text-2); }
.rec-pill.paused .dot { background: var(--text-3); }
.timer { font-size: 72px; font-weight: 200; letter-spacing: -0.02em; line-height: 1; }
.device { display: flex; align-items: center; gap: 6px; }
.sep { opacity: 0.5; }
.saved { display: inline-flex; align-items: center; gap: 4px; color: var(--success); }
.banners { display: flex; flex-direction: column; gap: var(--s2); width: min(520px, 100%); }
.banner { display: flex; align-items: center; gap: var(--s2); padding: 8px 8px 8px 12px; border-radius: var(--r-sm); background: var(--accent-soft); }
.banner > span { flex: 1; }
.banner.warning { background: var(--warning-soft); }
.banner.danger { background: var(--danger-soft); }
.banner.warning > svg { color: var(--warning); }
.banner.danger > svg { color: var(--danger); }
.banner.info > svg { color: var(--accent); }
.rec-actions { display: flex; gap: var(--s3); margin-top: var(--s3); }

/* processing */
.processing { flex: 1; display: flex; flex-direction: column; gap: var(--s4); min-height: 0; max-width: 760px; width: 100%; margin: 0 auto; padding-top: var(--s4); }
.proc-head { display: flex; justify-content: space-between; align-items: flex-start; gap: var(--s4); }
.stages { list-style: none; margin: 0; padding: 0; display: flex; flex-wrap: wrap; gap: var(--s2) var(--s5); }
.stages li { display: inline-flex; align-items: center; gap: 8px; color: var(--text-3); }
.stages li.done { color: var(--text-2); }
.stages li.current { color: var(--text); font-weight: 600; }
.mark { width: 16px; height: 16px; border-radius: 50%; border: 1.5px solid var(--border-strong); display: grid; place-items: center; }
.done .mark { background: var(--success); border-color: var(--success); color: var(--surface); }
.current .mark { border-color: var(--accent); background: var(--accent-soft); }
.bar-row { display: flex; align-items: center; gap: var(--s4); }
.bar { flex: 1; height: 8px; border-radius: 4px; background: var(--meter-bg); overflow: hidden; }
.fill { height: 100%; background: var(--accent); border-radius: 4px; transition: width var(--dur) var(--ease); }
.eta { min-width: 130px; text-align: right; }
.partial { flex: 1; min-height: 0; overflow: auto; padding: var(--s4) var(--s5); background: var(--surface); border: 1px solid var(--border); border-radius: var(--r-md); line-height: 1.6; }
.partial p + p { margin-top: var(--s3); }

/* result */
.result { flex: 1; min-height: 0; display: flex; flex-direction: column; gap: var(--s3); padding-top: var(--s2); }
.res-head { display: flex; justify-content: space-between; align-items: flex-start; gap: var(--s4); flex-wrap: wrap; }
.titles { min-width: 0; }
.titles h2 { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 420px; }
.meta { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; margin-top: 2px; }
.lang-chip { border: 0; cursor: default; }
.lang-chip:hover { background: var(--accent-soft); filter: brightness(0.96); }
.actions { display: flex; gap: var(--s2); align-items: center; }
.editor {
  flex: 1; min-height: 0; resize: none; padding: var(--s4) var(--s5); border-radius: var(--r-md);
  border: 1px solid var(--border); background: var(--surface); line-height: 1.65; font-size: 15px; box-shadow: var(--shadow-1);
}
.editor:focus-visible { outline: 2px solid var(--focus); outline-offset: 0; }

/* error */
.error { flex: 1; display: grid; place-items: center; }
.err-card { max-width: 520px; width: 100%; display: flex; flex-direction: column; gap: var(--s3); align-items: flex-start; }
.err-icon { width: 48px; height: 48px; border-radius: 50%; display: grid; place-items: center; background: var(--danger-soft); color: var(--danger); }
.err-actions { display: flex; gap: var(--s2); margin-top: var(--s2); }
.details { width: 100%; display: flex; flex-direction: column; gap: var(--s2); align-items: flex-start; }
.details pre { margin: 0; width: 100%; max-height: 140px; overflow: auto; padding: var(--s3); border-radius: var(--r-sm); background: var(--surface-2); white-space: pre-wrap; overflow-wrap: anywhere; }
</style>
