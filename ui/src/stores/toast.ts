import { defineStore } from 'pinia'
import { ref } from 'vue'

export interface Toast {
  id: number
  kind: 'info' | 'success' | 'error'
  message: string
  action?: { label: string; run: () => void }
}

export const useToastStore = defineStore('toast', () => {
  const toasts = ref<Toast[]>([])
  let next = 1
  const timers = new Map<number, ReturnType<typeof setTimeout>>()

  function dismiss(id: number) {
    toasts.value = toasts.value.filter((t) => t.id !== id)
    const t = timers.get(id)
    if (t) clearTimeout(t)
    timers.delete(id)
  }

  function push(kind: Toast['kind'], message: string, opts: { action?: Toast['action']; ms?: number } = {}) {
    const id = next++
    toasts.value = [...toasts.value.slice(-3), { id, kind, message, action: opts.action }]
    // Auto-dismiss is a one-shot timer started by a user-visible event; nothing runs while idle.
    const ms = opts.ms ?? (kind === 'error' ? 8000 : 3500)
    if (ms > 0) timers.set(id, setTimeout(() => dismiss(id), ms))
    return id
  }

  return {
    toasts,
    dismiss,
    info: (m: string, o?: { action?: Toast['action']; ms?: number }) => push('info', m, o),
    success: (m: string, o?: { action?: Toast['action']; ms?: number }) => push('success', m, o),
    error: (m: string, o?: { action?: Toast['action']; ms?: number }) => push('error', m, o),
  }
})
