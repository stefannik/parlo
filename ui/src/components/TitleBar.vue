<script setup lang="ts">
import { useAppStore } from '../stores/app'
import { useRecorderStore } from '../stores/recorder'
import Icon from './Icon.vue'

const app = useAppStore()
const recorder = useRecorderStore()
const tabs = [
  { id: 'transcribe', label: 'Transcribe', key: '⌘1' },
  { id: 'speak', label: 'Speak', key: '⌘2' },
] as const

function onKey(e: KeyboardEvent) {
  if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return
  app.tab = app.tab === 'transcribe' ? 'speak' : 'transcribe'
  ;(e.currentTarget as HTMLElement).querySelector<HTMLElement>('[aria-checked="true"]')?.focus()
}
</script>

<template>
  <header class="titlebar" data-tauri-drag-region>
    <div class="traffic-space" data-tauri-drag-region />
    <div class="seg" role="radiogroup" aria-label="Mode" @keydown="onKey">
      <button
        v-for="t in tabs"
        :key="t.id"
        type="button"
        role="radio"
        class="seg-item"
        :class="{ active: app.tab === t.id }"
        :aria-checked="app.tab === t.id"
        :tabindex="app.tab === t.id ? 0 : -1"
        :title="`${t.label} (${t.key})`"
        :data-testid="`tab-${t.id}`"
        @click="app.tab = t.id"
      >
        {{ t.label }}
        <span v-if="t.id === 'transcribe' && recorder.active" class="rec-dot" aria-label="Recording in progress" />
      </button>
    </div>
    <div class="right" data-tauri-drag-region>
      <button type="button" class="btn btn-ghost btn-icon" aria-label="Settings" title="Settings (⌘,)" data-testid="open-settings" @click="app.settingsOpen = true">
        <Icon name="gear" />
      </button>
    </div>
  </header>
</template>

<style scoped>
.titlebar {
  height: 52px;
  flex: none;
  display: grid;
  grid-template-columns: 1fr auto 1fr;
  align-items: center;
  padding: 0 var(--s3) 0 var(--s3);
  -webkit-app-region: drag;
}
.traffic-space { width: 76px; height: 100%; }
.right { display: flex; justify-content: flex-end; }
.right button, .seg { -webkit-app-region: no-drag; }
.seg {
  display: inline-flex;
  padding: 2px;
  border-radius: 10px;
  background: var(--surface-2);
  border: 1px solid var(--border);
}
.seg-item {
  position: relative;
  min-width: 108px;
  height: 28px;
  border: 0;
  border-radius: 8px;
  background: transparent;
  color: var(--text-2);
  font-weight: 500;
  transition: background var(--dur) var(--ease), color var(--dur) var(--ease), box-shadow var(--dur) var(--ease);
}
.seg-item:hover:not(.active) { color: var(--text); }
.seg-item.active { background: var(--surface); color: var(--text); box-shadow: var(--shadow-1); }
.rec-dot {
  display: inline-block; width: 7px; height: 7px; margin-left: 6px; border-radius: 50%;
  background: var(--danger); vertical-align: middle;
}
</style>
