import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { backend } from '../backend'
import type { InputDeviceDto, MicPermission, RecorderStateEvent, RecorderWarningDto, RecordingInfoDto } from '../bindings'

export interface RecorderBanner {
  id: number
  tone: 'info' | 'warning' | 'danger'
  text: string
}

export const useRecorderStore = defineStore('recorder', () => {
  const state = ref<'idle' | 'recording' | 'paused'>('idle')
  const recordingId = ref<string | null>(null)
  const elapsedMs = ref(0)
  const device = ref('')
  const rms = ref(0)
  const peak = ref(0)
  const gaps = ref(0)
  const banners = ref<RecorderBanner[]>([])
  const permission = ref<MicPermission>('unknown')
  const devices = ref<InputDeviceDto[]>([])
  const starting = ref(false)
  const lastError = ref<string | null>(null)
  let nextBanner = 1

  const active = computed(() => state.value !== 'idle')

  function onState(ev: RecorderStateEvent) {
    state.value = ev.state as typeof state.value
    recordingId.value = ev.id
    elapsedMs.value = ev.elapsed_ms
    device.value = ev.device
    rms.value = ev.rms
    peak.value = ev.peak
    gaps.value = ev.gaps
    if (ev.state === 'idle') {
      banners.value = []
      void refreshDevices() // the default input may have changed during the recording (AirPods in/out)
    }
  }

  function describe(w: RecorderWarningDto): RecorderBanner {
    const id = nextBanner++
    switch (w.kind) {
      case 'device_changed':
        return { id, tone: 'info', text: `Switched microphone to ${w.to}. Recording continues.` }
      case 'pinned_device_lost':
        return { id, tone: 'warning', text: `${w.pinned} is no longer available — recording from ${w.fallback} instead.` }
      case 'stream_restarted':
        return { id, tone: 'info', text: 'The audio input was restarted. Recording continues.' }
      case 'resumed_after_sleep':
        return { id, tone: 'info', text: `Recording resumed after the Mac woke up (${Math.round(w.gap_ms / 1000)} s were not captured).` }
      case 'low_disk':
        return { id, tone: 'warning', text: `Low disk space (${w.free_mb} MB free). Recording will stop automatically if it runs out.` }
      case 'silent_input':
        return { id, tone: 'warning', text: 'No sound is coming from the microphone. Check that it is not muted and the right input is selected.' }
      case 'clipping':
        return { id, tone: 'warning', text: 'The input is clipping — move back from the microphone or lower the input volume.' }
      case 'stopped':
        return { id, tone: 'danger', text: `Recording stopped: ${w.reason}. What was captured so far is saved.` }
    }
  }
  function onWarning(w: RecorderWarningDto) {
    const b = describe(w)
    // collapse repeats of the same warning text
    banners.value = [...banners.value.filter((x) => x.text !== b.text), b].slice(-3)
  }
  function dismissBanner(id: number) {
    banners.value = banners.value.filter((b) => b.id !== id)
  }

  async function refreshPermission() {
    permission.value = await backend().micStatus()
  }
  async function refreshDevices() {
    try {
      devices.value = await backend().inputDevices()
    } catch {
      devices.value = []
    }
  }
  async function requestPermission() {
    permission.value = await backend().micRequest()
    return permission.value
  }

  async function start(deviceId?: string | null): Promise<RecordingInfoDto | null> {
    if (state.value !== 'idle' || starting.value) return null
    starting.value = true
    lastError.value = null
    try {
      const info = await backend().recordingStart(deviceId ?? null)
      permission.value = 'granted'
      return info
    } catch (e) {
      const err = e as { code?: string; message?: string }
      if (err.code === 'mic_permission') permission.value = 'denied'
      lastError.value = err.message ?? 'Could not start recording.'
      return null
    } finally {
      starting.value = false
    }
  }
  const pause = () => backend().recordingPause()
  const resume = () => backend().recordingResume()
  const togglePause = () => (state.value === 'paused' ? resume() : state.value === 'recording' ? pause() : Promise.resolve())
  const stop = () => backend().recordingStop()
  async function discard() {
    const id = recordingId.value
    if (id) await backend().recordingDiscard(id)
  }

  /** Called once at startup: reattach to a recording the core is already making (window was closed/reopened). */
  async function hydrate() {
    await Promise.all([refreshPermission(), refreshDevices()])
  }

  return {
    state,
    recordingId,
    elapsedMs,
    device,
    rms,
    peak,
    gaps,
    banners,
    permission,
    devices,
    starting,
    lastError,
    active,
    onState,
    onWarning,
    dismissBanner,
    refreshPermission,
    refreshDevices,
    requestPermission,
    start,
    pause,
    resume,
    togglePause,
    stop,
    discard,
    hydrate,
  }
})
