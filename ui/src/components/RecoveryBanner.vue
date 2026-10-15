<script setup lang="ts">
import { backend } from '../backend'
import type { RecoverableDto } from '../bindings'
import { formatClock } from '../lib/format'
import { useAppStore } from '../stores/app'
import { useSpeakStore } from '../stores/speak'
import { useToastStore } from '../stores/toast'
import { useTranscribeStore } from '../stores/transcribe'
import Icon from './Icon.vue'

const app = useAppStore()
const transcribe = useTranscribeStore()
const speak = useSpeakStore()
const toast = useToastStore()

function title(r: RecoverableDto) {
  if (r.kind === 'recording' && r.saved_on_quit) return `A recording (${formatClock(r.duration_ms)}, ${new Date(r.started_at).toLocaleString()}) was saved when you quit Parlo — it hasn’t been transcribed yet.`
  if (r.kind === 'recording') return `An interrupted recording (${formatClock(r.duration_ms)}, ${new Date(r.started_at).toLocaleString()}) was recovered.`
  if (r.kind === 'stt_job') return `Transcription of “${r.source_name}” was interrupted.`
  return `Speech generation was interrupted: “${r.text_preview.slice(0, 60)}${r.text_preview.length > 60 ? '…' : ''}”`
}

async function transcribeRecovered(r: Extract<RecoverableDto, { kind: 'recording' }>) {
  app.recoverables = app.recoverables.filter((x) => x !== r)
  await transcribe.transcribeRecording(r.id, r.wav_path, 'Recovered recording')
}
async function saveRecovered(r: Extract<RecoverableDto, { kind: 'recording' }>) {
  const dest = await backend().saveFile('Recovered recording.wav', ['wav'])
  if (!dest) return
  try {
    await backend().exportRecording(r.id, 'wav', dest)
    toast.success('Recording saved')
  } catch (e) {
    toast.error(`Couldn't save: ${(e as Error).message}`)
  }
}
async function resume(r: Extract<RecoverableDto, { kind: 'stt_job' | 'tts_job' }>) {
  app.recoverables = app.recoverables.filter((x) => x !== r)
  if (r.kind === 'stt_job') await transcribe.resumeJob(r.job_id, r.source_name)
  else await speak.resumeJob(r.job_id)
}
</script>

<template>
  <div v-if="app.recoverables.length" class="stack" role="region" aria-label="Recovered work">
    <div v-for="(r, i) in app.recoverables" :key="i" class="banner" data-testid="recovery-banner">
      <Icon name="info" :size="16" />
      <span class="text">{{ title(r) }}</span>
      <template v-if="r.kind === 'recording'">
        <button type="button" class="btn btn-primary" @click="transcribeRecovered(r)">Transcribe</button>
        <button type="button" class="btn" @click="saveRecovered(r)">Save audio…</button>
      </template>
      <button v-else type="button" class="btn btn-primary" @click="resume(r)">Resume</button>
      <button type="button" class="btn btn-ghost" @click="app.dismissRecoverable(r)">Discard</button>
    </div>
  </div>
</template>

<style scoped>
.stack { display: flex; flex-direction: column; gap: var(--s2); padding: 0 var(--s5) var(--s2); }
.banner { display: flex; align-items: center; gap: var(--s3); padding: 8px 12px; border-radius: var(--r-md); background: var(--accent-soft); border: 1px solid var(--border); }
.banner > svg { color: var(--accent); flex: none; }
.text { flex: 1; min-width: 0; }
.btn { min-height: 28px; }
</style>
