<script setup lang="ts">
// Voice picker: a listbox popover with a ▶ preview per voice (bundled clips; plays through the asset protocol).
import { nextTick, onBeforeUnmount, ref } from 'vue'
import { backend } from '../backend'
import type { VoiceDto } from '../bindings'
import Icon from './Icon.vue'

const props = defineProps<{ voices: VoiceDto[]; modelValue: string }>()
const emit = defineEmits<{ 'update:modelValue': [string] }>()

const open = ref(false)
const root = ref<HTMLElement | null>(null)
const list = ref<HTMLElement | null>(null)
const previewing = ref<string | null>(null)
let audio: HTMLAudioElement | null = null

const current = () => props.voices.find((v) => v.id === props.modelValue)

async function show() {
  open.value = true
  await nextTick()
  ;(list.value?.querySelector('[aria-selected="true"] .pick') as HTMLElement | null)?.focus()
  document.addEventListener('pointerdown', outside, true)
}
function hide(refocus = false) {
  open.value = false
  document.removeEventListener('pointerdown', outside, true)
  stopPreview()
  if (refocus) root.value?.querySelector<HTMLElement>('.trigger')?.focus()
}
function outside(e: Event) {
  if (!root.value?.contains(e.target as Node)) hide()
}
function choose(id: string) {
  emit('update:modelValue', id)
  hide(true)
}
function stopPreview() {
  audio?.pause()
  audio = null
  previewing.value = null
}
function preview(v: VoiceDto) {
  if (!v.preview_path) return
  if (previewing.value === v.id) return stopPreview()
  stopPreview()
  audio = new Audio(backend().fileUrl(v.preview_path))
  previewing.value = v.id
  audio.onended = () => (previewing.value = null)
  audio.onerror = () => (previewing.value = null)
  void audio.play().catch(() => (previewing.value = null))
}
function onKey(e: KeyboardEvent) {
  const picks = Array.from(list.value?.querySelectorAll<HTMLElement>('.pick') ?? [])
  const i = picks.indexOf(document.activeElement as HTMLElement)
  if (e.key === 'ArrowDown') {
    e.preventDefault()
    picks[Math.min(picks.length - 1, i + 1)]?.focus()
  } else if (e.key === 'ArrowUp') {
    e.preventDefault()
    picks[Math.max(0, i - 1)]?.focus()
  } else if (e.key === 'Escape') {
    e.stopPropagation()
    hide(true)
  }
}
onBeforeUnmount(() => {
  stopPreview()
  document.removeEventListener('pointerdown', outside, true)
})
</script>

<template>
  <div ref="root" class="voice">
    <button type="button" class="btn trigger" aria-haspopup="listbox" :aria-expanded="open" data-testid="voice-picker" @click="open ? hide() : show()">
      <Icon name="speaker" :size="16" />
      <span>{{ current()?.label ?? 'Voice' }}</span>
      <Icon name="chevron" :size="14" />
    </button>
    <transition name="pop">
      <ul v-if="open" ref="list" class="list card" role="listbox" aria-label="Voice" @keydown="onKey">
        <li v-for="v in voices" :key="v.id" role="option" :aria-selected="v.id === modelValue" class="row" :class="{ sel: v.id === modelValue }">
          <button type="button" class="pick" :data-testid="`voice-${v.id}`" @click="choose(v.id)">
            <Icon v-if="v.id === modelValue" name="check" :size="14" />
            <span v-else class="gap" />
            {{ v.label }}
          </button>
          <button v-if="v.preview_path" type="button" class="btn btn-ghost btn-icon prev" :aria-label="`Preview ${v.label}`" tabindex="-1" @click="preview(v)">
            <Icon :name="previewing === v.id ? 'stop' : 'play'" :size="14" />
          </button>
        </li>
      </ul>
    </transition>
  </div>
</template>

<style scoped>
.voice { position: relative; }
.list { position: absolute; z-index: 30; bottom: calc(100% + 6px); left: 0; margin: 0; padding: 4px; list-style: none; min-width: 220px; max-height: 320px; overflow: auto; box-shadow: var(--shadow-2); }
.row { display: flex; align-items: center; border-radius: 6px; }
.row:hover { background: var(--surface-2); }
.pick { flex: 1; display: flex; align-items: center; gap: var(--s2); padding: 6px 10px; border: 0; background: transparent; text-align: left; border-radius: 6px; }
.gap { width: 14px; }
.prev { width: 28px; min-height: 28px; }
.pop-enter-active, .pop-leave-active { transition: opacity var(--dur) var(--ease), transform var(--dur) var(--ease); }
.pop-enter-from, .pop-leave-to { opacity: 0; transform: translateY(4px); }
</style>
