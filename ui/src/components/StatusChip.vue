<script setup lang="ts">
// Engine state chip (bottom right). The "unloads in 42 s" countdown re-renders on a timer that exists only while a
// countdown is visible (ready + unload scheduled); at idle nothing ticks.
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useAppStore } from '../stores/app'

const app = useAppStore()
const now = ref(Date.now())
let timer: ReturnType<typeof setInterval> | null = null

const active = computed(() => {
  const { stt, tts } = app.engines
  const list: Array<['Speech' | 'Voice', typeof stt]> = [
    ['Speech', stt],
    ['Voice', tts],
  ]
  return list.filter(([, e]) => e.state !== 'unloaded')
})

const label = computed(() => {
  if (!active.value.length) return 'Idle · no models in memory'
  return active.value
    .map(([n, e]) => {
      if (e.state === 'loading') return `${n} model loading…`
      if (e.unloadAt == null) return `${n} model loaded`
      const s = Math.max(0, Math.round((e.unloadAt - now.value) / 1000))
      return `${n} model loaded · unloads in ${s >= 60 ? `${Math.floor(s / 60)} min ${String(s % 60).padStart(2, '0')} s` : `${s} s`}`
    })
    .join(' · ')
})

const needsTick = computed(() => active.value.some(([, e]) => e.state === 'ready' && e.unloadAt != null))
watch(
  needsTick,
  (on) => {
    if (timer) clearInterval(timer)
    timer = null
    if (on) {
      now.value = Date.now()
      timer = setInterval(() => (now.value = Date.now()), 1000)
    }
  },
  { immediate: true },
)
onBeforeUnmount(() => timer && clearInterval(timer))
</script>

<template>
  <div class="status-chip" role="status" aria-live="off" data-testid="status-chip" :class="{ busy: active.length > 0 }">
    <span class="dot" />
    <span class="tabular">{{ label }}</span>
  </div>
</template>

<style scoped>
.status-chip {
  position: fixed; right: var(--s4); bottom: var(--s3); z-index: 5;
  display: inline-flex; align-items: center; gap: 8px;
  padding: 4px 12px; border-radius: 999px; font-size: 12px; color: var(--text-2);
  background: var(--surface); border: 1px solid var(--border); box-shadow: var(--shadow-1);
  pointer-events: none;
}
.dot { width: 7px; height: 7px; border-radius: 50%; background: var(--text-3); opacity: 0.6; }
.busy .dot { background: var(--success); opacity: 1; }
</style>
