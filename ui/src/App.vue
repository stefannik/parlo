<script setup lang="ts">
import ConfirmDialog from './components/ConfirmDialog.vue'
import RecoveryBanner from './components/RecoveryBanner.vue'
import SettingsSheet from './components/SettingsSheet.vue'
import StatusChip from './components/StatusChip.vue'
import TitleBar from './components/TitleBar.vue'
import ToastHost from './components/ToastHost.vue'
import Icon from './components/Icon.vue'
import { backend } from './backend'
import { useAppStore } from './stores/app'
import SpeakView from './views/SpeakView.vue'
import TranscribeView from './views/TranscribeView.vue'

const app = useAppStore()

async function quit(save: boolean) {
  app.quitPrompt = null
  if (save) await backend().quitConfirmed()
}
</script>

<template>
  <div class="shell">
    <TitleBar />
    <RecoveryBanner />
    <main class="main">
      <TranscribeView v-show="app.tab === 'transcribe'" />
      <SpeakView v-show="app.tab === 'speak'" />
    </main>
    <StatusChip />
    <SettingsSheet />
    <ConfirmDialog />
    <ToastHost />

    <transition name="fade">
      <div v-if="app.dropHover" class="drop-overlay" aria-hidden="true" data-testid="drop-overlay">
        <div class="drop-msg"><Icon name="upload" :size="28" /> Drop to {{ 'open' }}</div>
      </div>
    </transition>

    <transition name="fade">
      <div v-if="app.quitPrompt" class="scrim" @keydown.esc="app.quitPrompt = null">
        <div class="dialog card" role="alertdialog" aria-modal="true" aria-labelledby="q-title" data-testid="quit-dialog">
          <h2 id="q-title">{{ app.quitPrompt.recording ? 'Stop recording and quit?' : 'Quit Parlo?' }}</h2>
          <p class="muted">
            <template v-if="app.quitPrompt.recording">The recording so far will be saved, and you can transcribe it next time you open Parlo.</template>
            <template v-else>Work in progress will be cancelled; Parlo can resume it next time.</template>
          </p>
          <div class="actions">
            <button type="button" class="btn" @click="quit(false)">Cancel</button>
            <button type="button" class="btn btn-primary" data-testid="quit-confirm" @click="quit(true)">{{ app.quitPrompt.recording ? 'Stop and save, then quit' : 'Quit' }}</button>
          </div>
        </div>
      </div>
    </transition>
  </div>
</template>

<style scoped>
.shell { height: 100%; display: flex; flex-direction: column; min-height: 0; }
.main { flex: 1; min-height: 0; position: relative; }
.main > * { position: absolute; inset: 0; }
.drop-overlay { position: fixed; inset: 8px; z-index: 35; display: grid; place-items: center; border: 2px dashed var(--accent); border-radius: var(--r-lg); background: var(--accent-soft); pointer-events: none; }
.drop-msg { display: inline-flex; gap: 10px; align-items: center; padding: 10px 20px; border-radius: 999px; background: var(--surface); box-shadow: var(--shadow-2); font-weight: 600; color: var(--accent); }
.scrim { position: fixed; inset: 0; z-index: 70; display: grid; place-items: center; background: var(--scrim); }
.dialog { width: min(440px, calc(100vw - 48px)); padding: var(--s5); box-shadow: var(--shadow-2); }
h2 { font-size: 16px; margin-bottom: var(--s2); }
.actions { display: flex; justify-content: flex-end; gap: var(--s2); margin-top: var(--s5); }
.fade-enter-active, .fade-leave-active { transition: opacity var(--dur) var(--ease); }
.fade-enter-from, .fade-leave-to { opacity: 0; }
</style>
