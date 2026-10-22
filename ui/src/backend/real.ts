// Real backend: the generated, typed bindings (`../bindings`, produced by tauri-specta from the Rust DTOs) plus the
// handful of native-shell plugin calls (dialogs, asset URLs, drag-drop). Nothing here talks to the network.
import { Channel, convertFileSrc } from '@tauri-apps/api/core'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import { open, save } from '@tauri-apps/plugin-dialog'
import { commands, events } from '../bindings'
import type { IpcError } from '../bindings'
import { BackendError, type Backend, type BackendEvents, type FilePickKind, type Unlisten } from './types'

type Res<T> = { status: 'ok'; data: T } | { status: 'error'; error: IpcError }

function unwrap<T>(r: Res<T>): T {
  if (r.status === 'ok') return r.data
  throw new BackendError(r.error)
}

const MEDIA_EXT = ['wav', 'mp3', 'm4a', 'aac', 'flac', 'ogg', 'oga', 'opus', 'aiff', 'aif', 'caf', 'mp4', 'mov', 'm4v', 'mkv', 'webm', 'avi', 'wma', 'amr', '3gp']
const DOC_EXT = ['txt', 'md', 'markdown', 'docx', 'pdf', 'epub']

const EVENT_NAMES: { [K in keyof BackendEvents]: keyof typeof events } = {
  'recorder:state': 'recorderStateEvent',
  'recorder:warning': 'recorderWarningEvent',
  'models:state': 'modelsStateEvent',
  'jobs:changed': 'jobChangedEvent',
  'menu:action': 'menuActionEvent',
  'quit:requested': 'quitRequestedEvent',
}

export class RealBackend implements Backend {
  readonly isMock = false

  appInfo = () => commands.appInfo()
  settingsGet = () => commands.settingsGet()
  settingsUpdate = async (patch: Parameters<Backend['settingsUpdate']>[0]) => unwrap(await commands.settingsUpdate(patch))

  micStatus = () => commands.micPermissionStatus()
  micRequest = async () => unwrap(await commands.micPermissionRequest())
  openPrivacySettings = async (kind: 'microphone') => void unwrap(await commands.openPrivacySettings(kind))
  inputDevices = async () => unwrap(await commands.inputDevicesList())

  recordingStart = async (deviceId?: string | null) => unwrap(await commands.recordingStart(deviceId ?? null))
  recordingPause = async () => void unwrap(await commands.recordingPause())
  recordingResume = async () => void unwrap(await commands.recordingResume())
  recordingStop = async () => unwrap(await commands.recordingStop())
  recordingDiscard = async (id: string) => void unwrap(await commands.recordingDiscard(id))

  recoverables = async () => unwrap(await commands.recoverablesList())
  recoverableDiscard = async (kind: 'recording' | 'stt_job' | 'tts_job', id: string) => void unwrap(await commands.recoverableDiscard(kind, id))

  async transcribe(req: Parameters<Backend['transcribe']>[0], onEvent: Parameters<Backend['transcribe']>[1]) {
    const ch = new Channel<Parameters<Backend['transcribe']>[1] extends (e: infer E) => void ? E : never>()
    ch.onmessage = onEvent
    return unwrap(await commands.transcribeStart(req, ch))
  }
  async sttResume(jobId: string, onEvent: Parameters<Backend['sttResume']>[1]) {
    const ch = new Channel<Parameters<Backend['sttResume']>[1] extends (e: infer E) => void ? E : never>()
    ch.onmessage = onEvent
    return unwrap(await commands.sttResume(jobId, ch))
  }
  jobCancel = (jobId: number) => commands.jobCancel(jobId)

  async tts(req: Parameters<Backend['tts']>[0], onEvent: Parameters<Backend['tts']>[1]) {
    const ch = new Channel<Parameters<Backend['tts']>[1] extends (e: infer E) => void ? E : never>()
    ch.onmessage = onEvent
    return unwrap(await commands.ttsStart(req, ch))
  }
  async ttsResume(jobId: string, onEvent: Parameters<Backend['ttsResume']>[1]) {
    const ch = new Channel<Parameters<Backend['ttsResume']>[1] extends (e: infer E) => void ? E : never>()
    ch.onmessage = onEvent
    return unwrap(await commands.ttsResume(jobId, ch))
  }
  voices = () => commands.voicesList()

  importText = async (path: string) => unwrap(await commands.textImport(path))
  importFull = async (importId: string) => unwrap(await commands.textImportFull(importId))

  exportTts = async (outputId: string, format: Parameters<Backend['exportTts']>[1], dest: string) => unwrap(await commands.exportTts(outputId, format, dest))
  exportTranscript = async (text: string, format: Parameters<Backend['exportTranscript']>[1], dest: string, meta: Parameters<Backend['exportTranscript']>[3]) =>
    unwrap(await commands.exportTranscript(text, format, dest, meta))
  exportRecording = async (id: string, format: Parameters<Backend['exportRecording']>[1], dest: string) => unwrap(await commands.exportRecording(id, format, dest))
  exportDownload = async (id: string, format: Parameters<Backend['exportDownload']>[1], dest: string) => unwrap(await commands.exportDownload(id, format, dest))
  downloadsInfo = async () => unwrap(await commands.downloadsInfo())

  workspaceGet = () => commands.workspaceGet()
  workspaceUpdate = async (patch: Parameters<Backend['workspaceUpdate']>[0]) => void unwrap(await commands.workspaceUpdate(patch))

  storageUsage = async () => unwrap(await commands.storageUsage())
  storageClear = async (kind: 'outputs' | 'cache') => unwrap(await commands.storageClear(kind))

  reveal = async (path: string) => void unwrap(await commands.revealInFinder(path))
  setVisible = (visible: boolean) => commands.uiVisibility(visible)
  uiReady = () => commands.uiReady()
  diagnostics = () => commands.diagnosticsText()
  quitConfirmed = async () => void unwrap(await commands.quitConfirmed())

  async on<K extends keyof BackendEvents>(event: K, cb: (payload: BackendEvents[K]) => void): Promise<Unlisten> {
    const ev = events[EVENT_NAMES[event]] as unknown as { listen: (h: (e: { payload: BackendEvents[K] }) => void) => Promise<Unlisten> }
    return ev.listen((e) => cb(e.payload))
  }

  async pickFile(kind: FilePickKind) {
    const filters =
      kind === 'media' ? [{ name: 'Audio & video', extensions: MEDIA_EXT }] : kind === 'document' ? [{ name: 'Documents', extensions: DOC_EXT }] : undefined
    const r = await open({ multiple: false, directory: false, filters })
    return typeof r === 'string' ? r : null
  }
  async saveFile(defaultName: string, extensions: string[]) {
    return save({ defaultPath: defaultName, filters: [{ name: extensions.join(', ').toUpperCase(), extensions }] })
  }
  fileUrl = (path: string) => convertFileSrc(path)

  async onFileDrop(cb: (paths: string[]) => void, onHover?: (over: boolean) => void): Promise<Unlisten> {
    return getCurrentWebview().onDragDropEvent((e) => {
      const p = e.payload
      if (p.type === 'enter' || p.type === 'over') onHover?.(true)
      else if (p.type === 'leave') onHover?.(false)
      else if (p.type === 'drop') {
        onHover?.(false)
        cb(p.paths)
      }
    })
  }

  async copyText(text: string) {
    // WKWebView clipboard API (needs a user gesture + secure context); the textarea/execCommand path is the fallback
    // if the custom-protocol origin isn't treated as secure (verify on the Mac: docs/MAC_HANDOFF.md C9).
    try {
      await navigator.clipboard.writeText(text)
      return
    } catch {
      const ta = document.createElement('textarea')
      ta.value = text
      ta.style.position = 'fixed'
      ta.style.opacity = '0'
      document.body.appendChild(ta)
      ta.select()
      const ok = document.execCommand('copy')
      ta.remove()
      if (!ok) throw new Error('Copying to the clipboard is not available')
    }
  }
}
