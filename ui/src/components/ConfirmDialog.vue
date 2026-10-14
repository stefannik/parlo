<script setup lang="ts">
import { nextTick, ref, watch } from 'vue'
import { useAppStore } from '../stores/app'

const app = useAppStore()
const confirmBtn = ref<HTMLButtonElement | null>(null)

watch(
  () => app.confirm,
  async (c) => {
    if (c) {
      await nextTick()
      confirmBtn.value?.focus()
    }
  },
)
function onKey(e: KeyboardEvent) {
  if (e.key === 'Escape') {
    e.stopPropagation()
    app.answerConfirm(false)
  }
}
</script>

<template>
    <div v-if="app.confirm" class="scrim" @keydown="onKey" @click.self="app.answerConfirm(false)">
      <div class="dialog card" role="alertdialog" aria-modal="true" aria-labelledby="cd-title" aria-describedby="cd-body" data-testid="confirm-dialog">
        <h2 id="cd-title">{{ app.confirm.title }}</h2>
        <p id="cd-body" class="muted">{{ app.confirm.body }}</p>
        <div class="actions">
          <button type="button" class="btn" data-testid="confirm-cancel" @click="app.answerConfirm(false)">{{ app.confirm.cancelLabel ?? 'Cancel' }}</button>
          <button ref="confirmBtn" type="button" class="btn" :class="app.confirm.danger ? 'btn-danger' : 'btn-primary'" data-testid="confirm-ok" @click="app.answerConfirm(true)">
            {{ app.confirm.confirmLabel }}
          </button>
        </div>
      </div>
    </div>
</template>

<style scoped>
.scrim { position: fixed; inset: 0; z-index: 60; display: grid; place-items: center; background: var(--scrim); }
.dialog { width: min(420px, calc(100vw - 48px)); padding: var(--s5); box-shadow: var(--shadow-2); }
h2 { font-size: 16px; margin-bottom: var(--s2); }
.actions { display: flex; justify-content: flex-end; gap: var(--s2); margin-top: var(--s5); }
.scrim { animation: fade-in var(--dur) var(--ease); }
@keyframes fade-in { from { opacity: 0; } }
</style>
