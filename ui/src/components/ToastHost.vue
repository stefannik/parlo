<script setup lang="ts">
import { useToastStore } from '../stores/toast'
import Icon from './Icon.vue'

const toasts = useToastStore()
</script>

<template>
  <div class="toast-host" role="region" aria-label="Notifications" aria-live="polite">
    <div v-for="t in toasts.toasts" :key="t.id" class="toast" :class="t.kind" role="status" data-testid="toast">
        <Icon :name="t.kind === 'error' ? 'warn' : t.kind === 'success' ? 'check' : 'info'" :size="16" />
        <span class="msg">{{ t.message }}</span>
        <button v-if="t.action" type="button" class="btn btn-ghost toast-action" @click="t.action.run(); toasts.dismiss(t.id)">{{ t.action.label }}</button>
        <button type="button" class="btn btn-ghost btn-icon x" aria-label="Dismiss" @click="toasts.dismiss(t.id)"><Icon name="x" :size="14" /></button>
      </div>
  </div>
</template>

<style scoped>
.toast-host { position: fixed; left: 50%; bottom: 56px; transform: translateX(-50%); z-index: 50; display: flex; flex-direction: column; gap: var(--s2); align-items: center; width: min(560px, calc(100vw - 32px)); pointer-events: none; }
.toast {
  pointer-events: auto; display: flex; align-items: center; gap: var(--s3); max-width: 100%;
  padding: 8px 8px 8px 14px; border-radius: var(--r-md); background: var(--surface); border: 1px solid var(--border-strong); box-shadow: var(--shadow-2);
}
.toast.error { border-color: var(--danger); }
.toast.error > svg { color: var(--danger); }
.toast.success > svg { color: var(--success); }
.toast.info > svg { color: var(--accent); }
.msg { flex: 1; min-width: 0; overflow-wrap: anywhere; user-select: text; -webkit-user-select: text; }
.toast-action { color: var(--accent-text); min-height: 28px; padding: 0 var(--s3); }
.x { width: 28px; min-height: 28px; }
.toast { animation: toast-in var(--dur) var(--ease); }
@keyframes toast-in { from { opacity: 0; transform: translateY(8px); } }
</style>
