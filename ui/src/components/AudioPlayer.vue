<script setup lang="ts">
// Segmented audio player: one <audio> element walks a growing playlist of segment files (listen while it generates).
// Speed is playback-only (`playbackRate`, pitch preserved); exports are always 1×.
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { backend } from '../backend'
import { formatClock } from '../lib/format'
import Icon from './Icon.vue'

export interface PlayerSegment {
  path: string
  durationMs: number
}

const props = withDefaults(
  defineProps<{
    segments: PlayerSegment[]
    /** total known duration (when generation finished); otherwise the sum of ready segments */
    totalMs?: number | null
    generating?: boolean
    speed: number
    label?: string
    /** show the speed controls (Speak's bottom bar owns them there) */
    showSpeed?: boolean
  }>(),
  { totalMs: null, generating: false, label: 'Generated speech', showSpeed: true },
)
const emit = defineEmits<{ 'update:speed': [number]; started: [] }>()

const audio = ref<HTMLAudioElement | null>(null)
const index = ref(0)
const position = ref(0) // seconds within the current segment
const playing = ref(false)
const waiting = ref(false) // reached the end of the ready segments while more are coming
const failed = ref(false)

const starts = computed(() => {
  const out: number[] = []
  let acc = 0
  for (const s of props.segments) {
    out.push(acc)
    acc += s.durationMs
  }
  return out
})
const readyMs = computed(() => props.segments.reduce((a, s) => a + s.durationMs, 0))
const total = computed(() => (props.totalMs && !props.generating ? props.totalMs : readyMs.value))
const elapsed = computed(() => (starts.value[index.value] ?? 0) + position.value * 1000)
const src = computed(() => {
  const s = props.segments[index.value]
  return s ? backend().fileUrl(s.path) : ''
})

function apply() {
  const a = audio.value
  if (!a) return
  a.playbackRate = props.speed
  a.preservesPitch = true
}
watch(() => props.speed, apply)
watch(audio, apply)

// when a segment shows up while we were waiting at the end, continue
watch(
  () => props.segments.length,
  (n, old) => {
    if (old === 0 && n > 0) {
      emit('started')
    }
    if (waiting.value && index.value + 1 < n) {
      waiting.value = false
      index.value += 1
      void play()
    }
  },
)
// a new output replaces the playlist: rewind
watch(
  () => props.segments[0]?.path,
  () => {
    index.value = 0
    position.value = 0
    playing.value = false
    waiting.value = false
    failed.value = false
    audio.value?.pause()
  },
)

async function play() {
  const a = audio.value
  if (!a || !props.segments.length) return
  apply()
  try {
    await a.play()
    failed.value = false
  } catch {
    // autoplay policies or a missing file: surface as a paused state, never throw into the UI
    playing.value = false
    failed.value = true
  }
}
function pause() {
  audio.value?.pause()
}
async function toggle() {
  if (playing.value) return pause()
  // at the very end → restart
  if (!props.generating && index.value === props.segments.length - 1 && audio.value && audio.value.ended) {
    index.value = 0
    position.value = 0
    await Promise.resolve()
  }
  await play()
}
defineExpose({ toggle, play, pause })

function onEnded() {
  if (index.value + 1 < props.segments.length) {
    index.value += 1
    // the <audio> src swaps on the next tick; resume then
    void Promise.resolve().then(() => play())
  } else if (props.generating) {
    waiting.value = true
    playing.value = false
  } else {
    playing.value = false
    index.value = 0
    position.value = 0
  }
}

function onTime() {
  position.value = audio.value?.currentTime ?? 0
}

function seekTo(ms: number) {
  const segs = props.segments
  if (!segs.length) return
  let i = segs.length - 1
  for (let k = 0; k < segs.length; k++) {
    if (ms < starts.value[k]! + segs[k]!.durationMs) {
      i = k
      break
    }
  }
  const within = Math.max(0, (ms - starts.value[i]!) / 1000)
  const wasPlaying = playing.value
  if (i !== index.value) {
    index.value = i
    position.value = within
    void Promise.resolve().then(() => {
      if (audio.value) audio.value.currentTime = within
      if (wasPlaying) void play()
    })
  } else if (audio.value) {
    audio.value.currentTime = within
    position.value = within
  }
}
function onScrub(e: Event) {
  seekTo(Number((e.target as HTMLInputElement).value))
}

const speedText = computed(() => props.speed.toFixed(2))
function setSpeed(v: number) {
  if (!Number.isFinite(v)) return
  emit('update:speed', Math.min(2, Math.max(0.5, Math.round(v * 20) / 20)))
}

onBeforeUnmount(() => audio.value?.pause())
</script>

<template>
  <div class="player card" role="group" :aria-label="label" data-testid="player">
    <audio
      ref="audio"
      :src="src"
      preload="auto"
      @play="playing = true"
      @pause="playing = false"
      @ended="onEnded"
      @timeupdate="onTime"
      @loadedmetadata="apply"
      @error="failed = true"
    />
    <button type="button" class="btn btn-primary btn-icon play" :aria-label="playing ? 'Pause' : 'Play'" :disabled="!segments.length" data-testid="player-toggle" @click="toggle">
      <Icon :name="playing ? 'pause' : 'play'" :size="18" />
    </button>
    <div class="scrub">
      <input
        type="range"
        min="0"
        :max="Math.max(1, total)"
        step="100"
        :value="elapsed"
        :disabled="!segments.length"
        aria-label="Playback position"
        :aria-valuetext="`${formatClock(elapsed)} of ${formatClock(total)}`"
        data-testid="player-scrub"
        @input="onScrub"
      />
      <div class="times tabular small muted">
        <span data-testid="player-elapsed">{{ formatClock(elapsed) }}</span>
        <span v-if="waiting" class="chip">Waiting for the next part…</span>
        <span v-else-if="generating" class="chip chip-accent">Still generating</span>
        <span v-else-if="failed" class="chip">Can’t play this file</span>
        <span data-testid="player-total">{{ generating ? '≥ ' : '' }}{{ formatClock(total) }}</span>
      </div>
    </div>
    <div v-if="showSpeed" class="speed">
      <label class="small muted" for="player-speed">Speed</label>
      <input id="player-speed" type="range" min="0.5" max="2" step="0.05" :value="speed" aria-label="Playback speed" @input="setSpeed(Number(($event.target as HTMLInputElement).value))" />
      <input class="speed-num tabular" type="number" min="0.5" max="2" step="0.05" :value="speedText" aria-label="Playback speed value" @change="setSpeed(Number(($event.target as HTMLInputElement).value))" />
      <span class="small muted">×</span>
    </div>
  </div>
</template>

<style scoped>
.player { display: grid; grid-template-columns: auto 1fr auto; align-items: center; gap: var(--s4); padding: var(--s3) var(--s4); }
.play { width: 40px; height: 40px; border-radius: 50%; }
.scrub { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
.times { display: flex; justify-content: space-between; align-items: center; gap: var(--s2); }
.speed { display: flex; align-items: center; gap: var(--s2); }
.speed input[type='range'] { width: 110px; }
.speed-num { width: 56px; height: 28px; text-align: right; border: 1px solid var(--border); border-radius: var(--r-sm); background: var(--surface-2); padding: 0 6px; }
input[type='range'] { accent-color: var(--accent); width: 100%; margin: 0; }
</style>
