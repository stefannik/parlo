<script setup lang="ts">
// Button that opens a small menu (Export ▾). Keyboard: Enter/Space/ArrowDown open, arrows move, Esc closes.
import { nextTick, onBeforeUnmount, ref } from 'vue'
import Icon from './Icon.vue'

export interface MenuItem {
  id: string
  label: string
  hint?: string
  disabled?: boolean
}
const props = defineProps<{ label: string; items: MenuItem[]; icon?: string; primary?: boolean; disabled?: boolean; testid?: string; up?: boolean }>()
const emit = defineEmits<{ select: [string] }>()

const open = ref(false)
const root = ref<HTMLElement | null>(null)
const menu = ref<HTMLElement | null>(null)

function items() {
  return Array.from(menu.value?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]:not([disabled])') ?? [])
}
async function show() {
  if (props.disabled) return
  open.value = true
  await nextTick()
  items()[0]?.focus()
  document.addEventListener('pointerdown', outside, true)
}
function hide(refocus = false) {
  open.value = false
  document.removeEventListener('pointerdown', outside, true)
  if (refocus) root.value?.querySelector<HTMLElement>('button')?.focus()
}
function outside(e: Event) {
  if (!root.value?.contains(e.target as Node)) hide()
}
function pick(id: string) {
  hide(true)
  emit('select', id)
}
function onMenuKey(e: KeyboardEvent) {
  const els = items()
  const i = els.indexOf(document.activeElement as HTMLButtonElement)
  if (e.key === 'ArrowDown') {
    e.preventDefault()
    els[(i + 1) % els.length]?.focus()
  } else if (e.key === 'ArrowUp') {
    e.preventDefault()
    els[(i - 1 + els.length) % els.length]?.focus()
  } else if (e.key === 'Escape') {
    e.stopPropagation()
    hide(true)
  } else if (e.key === 'Tab') {
    hide()
  }
}
onBeforeUnmount(() => document.removeEventListener('pointerdown', outside, true))
</script>

<template>
  <div ref="root" class="menu-root">
    <button
      type="button"
      class="btn"
      :class="{ 'btn-primary': primary }"
      aria-haspopup="menu"
      :aria-expanded="open"
      :disabled="disabled"
      :data-testid="testid"
      @click="open ? hide() : show()"
      @keydown.down.prevent="show"
    >
      <Icon v-if="icon" :name="icon" :size="16" />
      {{ label }}
      <Icon name="chevron" :size="14" />
    </button>
    <transition name="pop">
      <div v-if="open" ref="menu" class="menu card" :class="{ up }" role="menu" @keydown="onMenuKey">
        <button v-for="it in items" :key="it.id" type="button" role="menuitem" class="item" :disabled="it.disabled" :data-testid="`menu-${it.id}`" @click="pick(it.id)">
          <span>{{ it.label }}</span>
          <span v-if="it.hint" class="small muted">{{ it.hint }}</span>
        </button>
      </div>
    </transition>
  </div>
</template>

<style scoped>
.menu-root { position: relative; display: inline-block; }
.menu { position: absolute; z-index: 30; top: calc(100% + 6px); right: 0; min-width: 200px; padding: 4px; box-shadow: var(--shadow-2); }
.menu.up { top: auto; bottom: calc(100% + 6px); }
.item { display: flex; width: 100%; justify-content: space-between; gap: var(--s4); align-items: center; padding: 7px 10px; border: 0; border-radius: 6px; background: transparent; text-align: left; }
.item:hover, .item:focus-visible { background: var(--accent); color: var(--on-accent); outline: none; }
.item:hover .muted, .item:focus-visible .muted { color: inherit; opacity: 0.85; }
.item[disabled] { opacity: 0.4; pointer-events: none; }
.pop-enter-active, .pop-leave-active { transition: opacity var(--dur) var(--ease), transform var(--dur) var(--ease); }
.pop-enter-from, .pop-leave-to { opacity: 0; transform: translateY(-4px); }
</style>
