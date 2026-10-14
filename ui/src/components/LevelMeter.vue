<script setup lang="ts">
// Horizontal level meter fed by the core's level events (≈10 Hz, only while recording and visible).
// No animation loop of its own: it re-renders when the props change.
import { computed } from 'vue'

const props = defineProps<{ rms: number; peak: number; active: boolean }>()
const BARS = 36
const level = computed(() => {
  // dB scale −60..0 mapped to 0..1 so quiet speech still moves the meter
  const db = 20 * Math.log10(Math.max(props.rms, 1e-4))
  return Math.max(0, Math.min(1, (db + 60) / 60))
})
const peakLevel = computed(() => {
  const db = 20 * Math.log10(Math.max(props.peak, 1e-4))
  return Math.max(0, Math.min(1, (db + 60) / 60))
})
const lit = computed(() => (props.active ? Math.round(level.value * BARS) : 0))
const peakBar = computed(() => (props.active ? Math.min(BARS - 1, Math.round(peakLevel.value * BARS)) : -1))
</script>

<template>
  <div class="meter" role="meter" aria-label="Input level" :aria-valuenow="Math.round(level * 100)" aria-valuemin="0" aria-valuemax="100" data-testid="level-meter">
    <span v-for="i in BARS" :key="i" class="bar" :class="{ on: i <= lit, hot: i > BARS * 0.85, peak: i - 1 === peakBar }" />
  </div>
</template>

<style scoped>
.meter { display: flex; gap: 3px; height: 28px; align-items: stretch; width: 100%; max-width: 420px; }
.bar { flex: 1; border-radius: 2px; background: var(--meter-bg); transition: background 60ms linear; }
.bar.on { background: var(--accent); }
.bar.on.hot { background: var(--danger); }
.bar.peak:not(.on) { background: var(--text-3); }
</style>
