<script setup lang="ts">
import { nextTick, ref, watch } from 'vue'
import { backend } from '../backend'
import type { IdleUnload, Performance, Retention, StorageEntryDto } from '../bindings'
import { formatBytes } from '../lib/format'
import { LANGUAGES } from '../lib/languages'
import { useAppStore } from '../stores/app'
import { useRecorderStore } from '../stores/recorder'
import { useSpeakStore } from '../stores/speak'
import { useToastStore } from '../stores/toast'
import Icon from './Icon.vue'

const app = useAppStore()
const rec = useRecorderStore()
const speak = useSpeakStore()
const toast = useToastStore()

const sections = [
  { id: 'general', label: 'General' },
  { id: 'recording', label: 'Recording' },
  { id: 'transcription', label: 'Transcription' },
  { id: 'speech', label: 'Speech' },
  { id: 'storage', label: 'Storage' },
  { id: 'about', label: 'About' },
] as const
const section = ref<(typeof sections)[number]['id']>('general')
const storage = ref<StorageEntryDto[]>([])
const dialog = ref<HTMLElement | null>(null)

watch(
  () => app.settingsOpen,
  async (open) => {
    if (!open) return
    section.value = 'general'
    await nextTick()
    dialog.value?.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]')?.focus()
    void rec.refreshDevices()
    void loadStorage()
  },
)
watch(section, (s) => s === 'storage' && loadStorage())

async function loadStorage() {
  try {
    storage.value = await backend().storageUsage()
  } catch {
    storage.value = []
  }
}
async function clear(kind: 'outputs' | 'cache') {
  const ok = await app.ask({
    title: kind === 'outputs' ? 'Delete generated speech?' : 'Clear the cache?',
    body: kind === 'outputs' ? 'All generated audio files are deleted. You can generate them again from the text.' : 'Temporary decoded audio is deleted. It is rebuilt when needed.',
    confirmLabel: kind === 'outputs' ? 'Delete' : 'Clear',
    danger: true,
  })
  if (!ok) return
  storage.value = await backend().storageClear(kind)
  toast.success(kind === 'outputs' ? 'Generated speech deleted' : 'Cache cleared')
}
function close() {
  app.settingsOpen = false
}
function onKey(e: KeyboardEvent) {
  if (e.key === 'Escape') {
    e.stopPropagation()
    close()
  } else if (e.key === 'Tab') {
    // keep focus inside the sheet
    const f = Array.from(dialog.value?.querySelectorAll<HTMLElement>('button, select, input, [tabindex="0"]') ?? []).filter((x) => !x.hasAttribute('disabled'))
    if (!f.length) return
    const first = f[0]!
    const last = f[f.length - 1]!
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault()
      last.focus()
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault()
      first.focus()
    }
  }
}
function tabKey(e: KeyboardEvent) {
  const i = sections.findIndex((s) => s.id === section.value)
  let n = i
  if (e.key === 'ArrowDown') n = (i + 1) % sections.length
  else if (e.key === 'ArrowUp') n = (i - 1 + sections.length) % sections.length
  else return
  e.preventDefault()
  section.value = sections[n]!.id
  void nextTick(() => dialog.value?.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]')?.focus())
}

const idleOptions: Array<[IdleUnload, string]> = [
  ['immediately', 'Immediately after use'],
  ['secs60', 'After 60 seconds'],
  ['mins5', 'After 5 minutes'],
  ['mins15', 'After 15 minutes'],
]
const retentionOptions: Array<[Retention, string]> = [
  ['day1', '1 day'],
  ['days7', '7 days'],
  ['days30', '30 days'],
  ['forever', 'Forever'],
]
const perfOptions: Array<[Performance, string, string]> = [
  ['balanced', 'Balanced', 'Leaves headroom for other apps'],
  ['maximum', 'Maximum', 'Uses every core; fastest'],
]

async function copyDiagnostics() {
  await backend().copyText(await backend().diagnostics())
  toast.success('Diagnostics copied')
}
function pick<T extends string>(e: Event): T {
  return (e.target as HTMLSelectElement).value as T
}
</script>

<template>
  <transition name="fade">
    <div v-if="app.settingsOpen" class="scrim" @click.self="close" @keydown="onKey">
      <div ref="dialog" class="sheet card" role="dialog" aria-modal="true" aria-label="Settings" data-testid="settings-sheet">
        <nav class="nav" role="tablist" aria-orientation="vertical" @keydown="tabKey">
          <button v-for="s in sections" :key="s.id" type="button" role="tab" :aria-selected="section === s.id" :tabindex="section === s.id ? 0 : -1" class="nav-item" :class="{ on: section === s.id }" :data-testid="`settings-${s.id}`" @click="section = s.id">
            {{ s.label }}
          </button>
        </nav>
        <div class="body" role="tabpanel">
          <button type="button" class="btn btn-ghost btn-icon close" aria-label="Close settings" data-testid="settings-close" @click="close"><Icon name="x" :size="16" /></button>

          <template v-if="app.settings">
            <div v-if="section === 'general'" class="group">
              <h2>General</h2>
              <div class="field">
                <label for="s-appearance">Appearance</label>
                <select id="s-appearance" :value="app.settings.appearance" @change="app.updateSettings({ appearance: pick($event) })">
                  <option value="system">Match system</option>
                  <option value="light">Light</option>
                  <option value="dark">Dark</option>
                </select>
              </div>
              <div class="field">
                <label for="s-idle">Unload models</label>
                <select id="s-idle" :value="app.settings.model_idle_timeout" @change="app.updateSettings({ model_idle_timeout: pick<IdleUnload>($event) })">
                  <option v-for="[v, l] in idleOptions" :key="v" :value="v">{{ l }}</option>
                </select>
                <p class="hint">Models stay in memory only while in use, so Parlo stays light when idle.</p>
              </div>
              <fieldset class="field">
                <legend>Performance</legend>
                <label v-for="[v, l, d] in perfOptions" :key="v" class="radio">
                  <input type="radio" name="perf" :value="v" :checked="app.settings.performance === v" @change="app.updateSettings({ performance: v })" />
                  <span><strong>{{ l }}</strong><br /><span class="muted small">{{ d }}</span></span>
                </label>
              </fieldset>
            </div>

            <div v-else-if="section === 'recording'" class="group">
              <h2>Recording</h2>
              <div class="field">
                <label for="s-device">Input device</label>
                <select id="s-device" :value="app.settings.input_device ?? ''" @change="app.updateSettings({ input_device: pick($event) })">
                  <option value="">System default</option>
                  <option v-for="d in rec.devices" :key="d.id" :value="d.id">{{ d.name }}{{ d.is_default ? ' (default)' : '' }}</option>
                </select>
              </div>
              <div class="field">
                <label for="s-ret">Keep recordings</label>
                <select id="s-ret" :value="app.settings.recording_retention" @change="app.updateSettings({ recording_retention: pick<Retention>($event) })">
                  <option v-for="[v, l] in retentionOptions" :key="v" :value="v">{{ l }}</option>
                </select>
                <p class="hint">Recordings that were never transcribed are never deleted automatically.</p>
              </div>
              <button type="button" class="btn" @click="app.info && backend().reveal(app.info.recordings_dir)"><Icon name="folder" :size="16" /> Open recordings folder</button>
            </div>

            <div v-else-if="section === 'transcription'" class="group">
              <h2>Transcription</h2>
              <div class="field">
                <label for="s-lang">Default language</label>
                <select id="s-lang" :value="app.settings.default_language" @change="app.updateSettings({ default_language: pick($event) })">
                  <option v-for="l in LANGUAGES" :key="l.code" :value="l.code">{{ l.name }}</option>
                </select>
              </div>
            </div>

            <div v-else-if="section === 'speech'" class="group">
              <h2>Speech</h2>
              <div class="field">
                <label for="s-voice">Default voice</label>
                <select id="s-voice" :value="app.settings.default_voice" @change="app.updateSettings({ default_voice: pick($event) })">
                  <option v-for="v in speak.voices" :key="v.id" :value="v.id">{{ v.label }}</option>
                </select>
              </div>
              <div class="field">
                <label for="s-speed">Default speed</label>
                <div class="inline">
                  <input id="s-speed" type="range" min="0.5" max="2" step="0.05" :value="app.settings.default_speed" @change="app.updateSettings({ default_speed: Number(($event.target as HTMLInputElement).value) })" />
                  <span class="tabular">{{ app.settings.default_speed.toFixed(2) }}×</span>
                </div>
                <p class="hint">Speed changes playback only. Exports are always 1×.</p>
              </div>
              <div class="field">
                <label for="s-quality">Quality</label>
                <select id="s-quality" :value="app.settings.tts_quality" @change="app.updateSettings({ tts_quality: pick($event) })">
                  <option value="standard">Standard</option>
                  <option value="fast">Fast</option>
                </select>
              </div>
            </div>

            <div v-else-if="section === 'storage'" class="group">
              <h2>Storage</h2>
              <ul class="usage">
                <li v-for="s in storage" :key="s.kind" :data-testid="`usage-${s.kind}`">
                  <span>{{ s.label }}</span>
                  <span class="tabular muted">{{ formatBytes(s.bytes) }}</span>
                  <button v-if="s.kind === 'outputs' || s.kind === 'cache'" type="button" class="btn" :disabled="s.bytes === 0" @click="clear(s.kind as 'outputs' | 'cache')">Clear</button>
                  <span v-else />
                </li>
              </ul>
              <p class="hint">Everything Parlo stores is on this Mac, in <span class="mono selectable">{{ app.info?.data_dir }}</span>.</p>
            </div>

            <div v-else class="group">
              <h2>About</h2>
              <p><strong>Parlo</strong> {{ app.info?.version }}</p>
              <p class="privacy"><Icon name="check" :size="14" /> Parlo never connects to the internet. All speech processing happens on this Mac.</p>
              <dl class="about selectable">
                <dt>Speech to text</dt><dd>{{ app.info?.stt_engine }}</dd>
                <dt>Text to speech</dt><dd>{{ app.info?.tts_engine }}</dd>
                <dt>Models</dt>
                <dd>
                  <div v-for="m in app.info?.models" :key="m.name" class="model"><span>{{ m.name }}</span><span class="muted small">{{ m.present ? formatBytes(m.size_bytes) : 'missing' }}</span></div>
                </dd>
              </dl>
              <p class="muted small">Speech recognition: Whisper (OpenAI, MIT) via whisper.cpp · Voice activity: Silero VAD (MIT) · Speech synthesis: Kyutai PocketTTS (CC-BY 4.0, model attribution: Kyutai) · Candle (Apache-2.0/MIT).</p>
              <button type="button" class="btn" data-testid="copy-diagnostics" @click="copyDiagnostics"><Icon name="copy" :size="16" /> Copy diagnostics</button>
            </div>
          </template>
        </div>
      </div>
    </div>
  </transition>
</template>

<style scoped>
.scrim { position: fixed; inset: 0; z-index: 40; display: grid; place-items: center; background: var(--scrim); }
.sheet { display: grid; grid-template-columns: 168px 1fr; width: min(720px, calc(100vw - 48px)); height: min(500px, calc(100vh - 64px)); box-shadow: var(--shadow-2); overflow: hidden; }
.nav { display: flex; flex-direction: column; gap: 2px; padding: var(--s3); background: var(--surface-2); border-right: 1px solid var(--border); }
.nav-item { text-align: left; padding: 7px 12px; border: 0; border-radius: var(--r-sm); background: transparent; color: var(--text-2); font-weight: 500; }
.nav-item:hover { background: var(--surface-hover); }
.nav-item.on { background: var(--accent); color: var(--on-accent); }
.body { position: relative; padding: var(--s5) var(--s6) var(--s5) var(--s5); overflow: auto; }
.close { position: absolute; top: 10px; right: 10px; width: 28px; min-height: 28px; }
h2 { font-size: 17px; margin-bottom: var(--s4); }
.group { display: flex; flex-direction: column; gap: var(--s4); align-items: flex-start; }
.field { display: flex; flex-direction: column; gap: 6px; border: 0; padding: 0; margin: 0; width: 100%; }
.field > label, legend { font-weight: 600; font-size: 13px; padding: 0; }
.hint { color: var(--text-2); font-size: 12px; margin: 0; }
select { appearance: none; -webkit-appearance: none; min-height: 30px; width: 260px; padding: 0 28px 0 12px; border-radius: var(--r-sm); border: 1px solid var(--border); background: var(--surface) url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='10' height='6' viewBox='0 0 10 6'%3E%3Cpath d='m1 1 4 4 4-4' fill='none' stroke='%23888' stroke-width='1.5' stroke-linecap='round'/%3E%3C/svg%3E") no-repeat right 10px center; }
.radio { display: flex; gap: var(--s3); align-items: flex-start; padding: 4px 0; }
.radio input { accent-color: var(--accent); margin-top: 3px; }
.inline { display: flex; gap: var(--s3); align-items: center; }
.inline input { accent-color: var(--accent); width: 200px; }
.usage { list-style: none; margin: 0; padding: 0; width: 100%; display: flex; flex-direction: column; gap: 2px; }
.usage li { display: grid; grid-template-columns: 1fr 90px 72px; align-items: center; gap: var(--s3); padding: 6px 0; border-bottom: 1px solid var(--border); }
.privacy { display: flex; align-items: center; gap: 6px; padding: 6px 12px; border-radius: var(--r-sm); background: var(--success-soft); }
.privacy svg { color: var(--success); }
.about { display: grid; grid-template-columns: 120px 1fr; gap: 6px var(--s3); margin: 0; width: 100%; }
.about dt { color: var(--text-2); }
.about dd { margin: 0; }
.model { display: flex; justify-content: space-between; gap: var(--s3); }
.fade-enter-active, .fade-leave-active { transition: opacity var(--dur) var(--ease); }
.fade-enter-from, .fade-leave-to { opacity: 0; }
</style>
