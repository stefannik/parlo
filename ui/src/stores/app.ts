import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { backend } from '../backend'
import type { AppInfo, ModelsStateEvent, RecoverableDto, SettingsDto, SettingsPatch } from '../bindings'
import { settingsPatch } from '../lib/settings'
import { useToastStore } from './toast'

export type Tab = 'transcribe' | 'speak'

export interface EngineStatus {
  state: 'unloaded' | 'loading' | 'ready'
  /** epoch ms at which the core will unload the model (derived from `unload_in_ms` on each event), or null */
  unloadAt: number | null
}

export interface ConfirmRequest {
  title: string
  body: string
  confirmLabel: string
  cancelLabel?: string
  danger?: boolean
  resolve: (ok: boolean) => void
}

export const useAppStore = defineStore('app', () => {
  const tab = ref<Tab>('transcribe')
  const settings = ref<SettingsDto | null>(null)
  const info = ref<AppInfo | null>(null)
  const settingsOpen = ref(false)
  /** which Settings section opens first (e.g. 'downloads' from a download error) */
  const settingsSection = ref<string | null>(null)
  const recoverables = ref<RecoverableDto[]>([])
  const engines = ref<{ stt: EngineStatus; tts: EngineStatus }>({
    stt: { state: 'unloaded', unloadAt: null },
    tts: { state: 'unloaded', unloadAt: null },
  })
  const dropHover = ref(false)
  /** bumped by ⌘E; the visible Export menu opens when it changes */
  const exportSignal = ref(0)
  const confirm = ref<ConfirmRequest | null>(null)
  const quitPrompt = ref<{ recording: boolean; job_running: boolean } | null>(null)

  const appearance = computed(() => settings.value?.appearance ?? 'system')

  function applyTheme() {
    const el = document.documentElement
    if (appearance.value === 'system') el.removeAttribute('data-theme')
    else el.setAttribute('data-theme', appearance.value)
  }

  async function load() {
    const b = backend()
    const [s, i, r] = await Promise.all([b.settingsGet(), b.appInfo(), b.recoverables()])
    settings.value = s
    info.value = i
    recoverables.value = r
    applyTheme()
  }

  async function updateSettings(patch: Partial<SettingsPatch>) {
    try {
      settings.value = await backend().settingsUpdate(settingsPatch(patch))
      applyTheme()
    } catch (e) {
      useToastStore().error(`Couldn't save the setting. ${(e as Error).message}`)
    }
  }

  function onModels(ev: ModelsStateEvent) {
    const key = ev.engine === 'tts' ? 'tts' : 'stt'
    engines.value[key] = {
      state: ev.state as EngineStatus['state'],
      unloadAt: ev.unload_in_ms != null ? Date.now() + ev.unload_in_ms : null,
    }
  }

  /** Promise-based confirmation sheet (never `confirm()`/`alert()`). */
  function ask(req: Omit<ConfirmRequest, 'resolve'>): Promise<boolean> {
    return new Promise((resolve) => {
      confirm.value?.resolve(false)
      confirm.value = { ...req, resolve }
    })
  }
  function answerConfirm(ok: boolean) {
    const c = confirm.value
    confirm.value = null
    c?.resolve(ok)
  }

  async function refreshRecoverables() {
    recoverables.value = await backend().recoverables()
  }
  async function dismissRecoverable(r: RecoverableDto) {
    const kind = r.kind === 'recording' ? 'recording' : r.kind === 'stt_job' ? 'stt_job' : 'tts_job'
    const id = r.kind === 'recording' ? r.id : r.job_id
    await backend().recoverableDiscard(kind, id)
    recoverables.value = recoverables.value.filter((x) => x !== r)
  }

  return {
    tab,
    settings,
    info,
    settingsOpen,
    settingsSection,
    recoverables,
    engines,
    dropHover,
    exportSignal,
    confirm,
    quitPrompt,
    load,
    updateSettings,
    applyTheme,
    onModels,
    ask,
    answerConfirm,
    refreshRecoverables,
    dismissRecoverable,
  }
})
