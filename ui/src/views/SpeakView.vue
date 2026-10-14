<script setup lang="ts">
import { computed } from 'vue'
import { backend } from '../backend'
import AudioPlayer from '../components/AudioPlayer.vue'
import Icon from '../components/Icon.vue'
import MenuButton from '../components/MenuButton.vue'
import VoicePicker from '../components/VoicePicker.vue'
import { formatClock, formatEta, formatListening } from '../lib/format'
import { useAppStore } from '../stores/app'
import { useSpeakStore } from '../stores/speak'

const app = useAppStore()
const sp = useSpeakStore()

const pct = computed(() => (sp.progress && sp.progress.totalChars > 0 ? Math.round((sp.progress.doneChars / sp.progress.totalChars) * 100) : null))
const busy = computed(() => sp.phase === 'generating')

async function openFile() {
  const p = await backend().pickFile('document')
  if (p) await sp.importFile(p)
}
const audioItems = [
  { id: 'wav', label: 'WAV', hint: 'lossless' },
  { id: 'm4a', label: 'M4A (AAC)', hint: 'small' },
  { id: 'mp3', label: 'MP3', hint: 'compatible' },
]
const stageText = computed(() => (sp.stageLabel === 'loading_model' ? 'Loading voice model…' : sp.stageLabel === 'preparing' ? 'Preparing text…' : 'Generating speech…'))
defineExpose({ openFile })
</script>

<template>
  <section class="view" aria-label="Speak" data-testid="speak-view" :data-phase="sp.phase">
    <div v-if="sp.imported" class="source-chip" data-testid="source-chip">
      <Icon name="file" :size="14" />
      <span class="name selectable">{{ sp.imported.file_name }}</span>
      <span v-if="sp.imported.pages" class="muted small">{{ sp.imported.pages }} pages</span>
      <span v-if="sp.imported.chapters" class="muted small">{{ sp.imported.chapters }} chapters</span>
      <span v-for="w in sp.imported.warnings" :key="w" class="chip warn-chip" data-testid="import-warning"><Icon name="info" :size="12" /> {{ w }}</span>
      <button type="button" class="btn btn-ghost btn-icon x" aria-label="Remove file" @click="sp.clear()"><Icon name="x" :size="14" /></button>
    </div>

    <div class="editor-wrap">
      <textarea
        class="editor"
        :value="sp.text"
        :readonly="sp.readOnly || busy"
        placeholder="Paste or type text — or drop a .txt, .md, .docx, .pdf or .epub"
        spellcheck="true"
        aria-label="Text to speak"
        data-testid="speak-editor"
        @input="sp.setText(($event.target as HTMLTextAreaElement).value)"
      />
      <div v-if="sp.readOnly" class="large-note small muted">Large document — showing the beginning. The whole document will be read.</div>
      <div v-if="sp.importing" class="overlay muted" role="status">Reading file…</div>
    </div>

    <div class="footer-row">
      <p class="muted small tabular counts" data-testid="counts">
        {{ sp.words.toLocaleString() }} words · {{ sp.chars.toLocaleString() }} characters · {{ formatListening(sp.minutes1x) }} at 1×<template v-if="Math.abs(sp.speed - 1) > 0.01"> · {{ formatListening(sp.minutesAtSpeed) }} at {{ sp.speed.toFixed(2).replace(/0$/, '') }}×</template>
      </p>
      <div class="row-actions">
        <button type="button" class="btn btn-ghost" :disabled="busy" data-testid="open-file" @click="openFile"><Icon name="folder" :size="16" /> Open file…</button>
        <button type="button" class="btn btn-ghost" :disabled="busy || (!sp.text && !sp.imported)" data-testid="clear-text" @click="sp.clear()">Clear</button>
      </div>
    </div>

    <!-- progress + player -->
    <div v-if="sp.phase !== 'compose'" class="output">
      <div v-if="sp.phase === 'generating'" class="progress card" role="status" aria-live="polite">
        <div class="p-row">
          <span class="p-label">{{ stageText }}</span>
          <span class="muted small tabular" data-testid="tts-eta">{{ pct != null ? `${pct}% · ${formatEta(sp.progress?.etaMs)}` : '' }}</span>
          <button type="button" class="btn" :disabled="sp.cancelling" data-testid="cancel-tts" @click="sp.cancel()">{{ sp.cancelling ? 'Cancelling…' : 'Cancel' }}</button>
        </div>
        <div class="bar" role="progressbar" :aria-valuenow="pct ?? undefined" aria-valuemin="0" aria-valuemax="100" aria-label="Generating speech"><div class="fill" :style="{ width: `${pct ?? 0}%` }" /></div>
      </div>
      <div v-if="sp.phase === 'error' && sp.error" class="err card" role="alert" data-testid="speak-error">
        <Icon name="warn" :size="18" />
        <div class="err-text">
          <strong>Couldn’t generate speech</strong>
          <p class="selectable">{{ sp.error.message }}</p>
        </div>
        <button type="button" class="btn btn-primary" data-testid="speak-retry" @click="sp.generate()"><Icon name="refresh" :size="16" /> Retry</button>
      </div>
      <AudioPlayer v-if="sp.hasOutput" v-model:speed="sp.speed" :show-speed="false" :segments="sp.segments" :total-ms="sp.durationMs" :generating="sp.phase === 'generating'" />
      <div v-if="sp.phase === 'result'" class="res-actions">
        <span v-if="sp.outOfDate" class="chip stale" data-testid="out-of-date"><Icon name="warn" :size="12" /> Out of date — the text or voice changed</span>
        <span v-else class="muted small tabular">{{ formatClock(sp.durationMs ?? 0) }}<template v-if="sp.cacheHit"> · reused from earlier</template> · Exports are always 1×</span>
        <span class="spacer" />
        <MenuButton label="Export" :items="audioItems" icon="download" up testid="export-audio" :signal="app.exportSignal" @select="(id) => sp.exportAs(id as 'wav' | 'm4a' | 'mp3')" />
        <button type="button" class="btn" data-testid="reveal" @click="sp.reveal()"><Icon name="folder" :size="16" /> Reveal in Finder</button>
      </div>
    </div>

    <!-- bottom bar -->
    <div class="bottom card">
      <VoicePicker v-model="sp.voice" :voices="sp.voices" />
      <div class="speed">
        <label class="small muted" for="speak-speed">Speed</label>
        <input id="speak-speed" type="range" min="0.5" max="2" step="0.05" :value="sp.speed" data-testid="speed-slider" @input="sp.speed = Number(($event.target as HTMLInputElement).value)" />
        <input class="num tabular" type="number" min="0.5" max="2" step="0.05" :value="sp.speed.toFixed(2)" aria-label="Speed value" @change="sp.speed = Math.min(2, Math.max(0.5, Number(($event.target as HTMLInputElement).value) || 1))" />
        <span class="small muted">×</span>
      </div>
      <span class="spacer" />
      <button type="button" class="btn btn-primary btn-lg" :disabled="!sp.canGenerate" data-testid="generate" title="Generate (⌘↩)" @click="sp.generate()">
        <Icon :name="sp.phase === 'result' ? 'refresh' : 'wave'" :size="18" /> {{ sp.phase === 'result' ? 'Regenerate' : 'Generate' }}
      </button>
    </div>
  </section>
</template>

<style scoped>
.view { height: 100%; display: flex; flex-direction: column; gap: var(--s3); padding: 0 var(--s5) var(--s5); min-height: 0; }
.source-chip { display: flex; align-items: center; gap: var(--s2); padding: 6px 6px 6px 12px; border-radius: var(--r-md); background: var(--surface); border: 1px solid var(--border); align-self: flex-start; max-width: 100%; }
.source-chip .name { font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 320px; }
.x { width: 24px; min-height: 24px; }
.warn-chip { background: var(--warning-soft); color: var(--text); }
.editor-wrap { position: relative; flex: 1; min-height: 120px; display: flex; flex-direction: column; }
.editor { flex: 1; min-height: 0; resize: none; padding: var(--s4) var(--s5); border-radius: var(--r-lg); border: 1px solid var(--border); background: var(--surface); font-size: 15px; line-height: 1.65; box-shadow: var(--shadow-1); }
.editor::placeholder { color: var(--text-3); }
.editor:focus-visible { outline: 2px solid var(--focus); outline-offset: 0; }
.editor[readonly] { color: var(--text-2); }
.large-note { position: absolute; left: 20px; bottom: 10px; padding: 2px 10px; border-radius: 999px; background: var(--surface-2); }
.overlay { position: absolute; inset: 0; display: grid; place-items: center; background: var(--scrim); border-radius: var(--r-lg); color: var(--on-accent); }
.footer-row { display: flex; align-items: center; justify-content: space-between; gap: var(--s3); }
.row-actions { display: flex; gap: var(--s1); }
.output { display: flex; flex-direction: column; gap: var(--s2); }
.progress { padding: var(--s3) var(--s4); display: flex; flex-direction: column; gap: var(--s2); }
.p-row { display: flex; align-items: center; gap: var(--s3); }
.p-label { flex: 1; font-weight: 500; }
.bar { height: 6px; border-radius: 3px; background: var(--meter-bg); overflow: hidden; }
.fill { height: 100%; background: var(--accent); transition: width var(--dur) var(--ease); }
.err { display: flex; align-items: center; gap: var(--s3); padding: var(--s3) var(--s4); border-color: var(--danger); }
.err > svg { color: var(--danger); flex: none; }
.err-text { flex: 1; min-width: 0; }
.res-actions { display: flex; align-items: center; gap: var(--s2); }
.spacer { flex: 1; }
.stale { background: var(--warning-soft); color: var(--text); }
.bottom { display: flex; align-items: center; gap: var(--s4); padding: var(--s3) var(--s4); }
.speed { display: flex; align-items: center; gap: var(--s2); }
.speed input[type='range'] { width: 140px; accent-color: var(--accent); }
.num { width: 60px; height: 28px; text-align: right; border: 1px solid var(--border); border-radius: var(--r-sm); background: var(--surface-2); padding: 0 6px; }
</style>
