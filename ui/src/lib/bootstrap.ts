// Startup + global interactions (menu actions, shortcuts, drag-drop, quit prompt, visibility).
import { backend } from '../backend'
import type { Unlisten } from '../backend/types'
import { classifyPath, baseName } from './format'
import { linkFromDrop, parseHttpUrl } from './url'
import { useAppStore } from '../stores/app'
import { useRecorderStore } from '../stores/recorder'
import { useSpeakStore } from '../stores/speak'
import { useToastStore } from '../stores/toast'
import { useTranscribeStore } from '../stores/transcribe'

/** Load persisted state and reattach to whatever the core is doing (recorder running while the window was closed, etc.). */
export async function hydrate() {
  const app = useAppStore()
  const rec = useRecorderStore()
  const tr = useTranscribeStore()
  const sp = useSpeakStore()
  await Promise.all([app.load(), rec.hydrate(), sp.loadVoices()])
  sp.applyDefaults()
  if (app.settings) tr.language = app.settings.default_language
  const ws = await backend().workspaceGet()
  // after a normal quit, open ready to record; the transcript stays one click away (DECISIONS C-026)
  if (ws.transcript && ws.transcript_parked) tr.parked = ws.transcript
  else if (ws.transcript) tr.restore(ws.transcript)
  if (ws.tts) sp.restore(ws.tts)
  // a download that was interrupted (quit / crash / failure): the link is still in the field, retrying is one click
  if (ws.pending_url && !ws.transcript) tr.urlDraft = ws.pending_url
}

/** Start "Transcribe from URL" for a link that was pasted or dropped on the empty Transcribe screen. */
export async function handleLink(link: string): Promise<boolean> {
  const app = useAppStore()
  const rec = useRecorderStore()
  const tr = useTranscribeStore()
  const url = parseHttpUrl(link)
  if (!url) return false
  if (app.settings && !app.settings.allow_downloads) return false
  if (rec.active || tr.phase !== 'empty') return false
  app.tab = 'transcribe'
  tr.urlDraft = url
  return tr.transcribeUrl(url)
}

export type Action = 'settings' | 'quit' | 'open' | 'record' | 'pause' | 'export' | 'copy_transcript' | 'tab_transcribe' | 'tab_speak' | 'generate'

export async function runAction(action: string) {
  const app = useAppStore()
  const rec = useRecorderStore()
  const tr = useTranscribeStore()
  const sp = useSpeakStore()
  // Dialogs own the keyboard: ignore global actions while one is open (except opening Settings from nowhere).
  if (app.confirm || (app.settingsOpen && action !== 'settings')) return
  switch (action as Action) {
    case 'settings':
      app.settingsOpen = !app.settingsOpen
      break
    case 'tab_transcribe':
      app.tab = 'transcribe'
      break
    case 'tab_speak':
      app.tab = 'speak'
      break
    case 'open': {
      if (app.tab === 'speak') {
        const p = await backend().pickFile('document')
        if (p) await sp.importFile(p)
      } else {
        const p = await backend().pickFile('media')
        if (p) await handlePath(p)
      }
      break
    }
    case 'record':
      app.tab = 'transcribe'
      await tr.toggleRecording()
      break
    case 'pause':
      if (rec.active) await rec.togglePause()
      break
    case 'export':
      if ((app.tab === 'transcribe' && tr.phase === 'result') || (app.tab === 'speak' && sp.phase === 'result')) app.exportSignal++
      break
    case 'copy_transcript':
      if (tr.phase === 'result') await tr.copy()
      break
    case 'generate':
      app.tab = 'speak'
      await sp.generate()
      break
  }
}

/** Route a dropped / opened path: audio & video → Transcribe, documents → Speak. */
export async function handlePath(path: string) {
  const app = useAppStore()
  const rec = useRecorderStore()
  const tr = useTranscribeStore()
  const sp = useSpeakStore()
  const toast = useToastStore()
  switch (classifyPath(path)) {
    case 'media':
      if (rec.active) return void toast.info('Stop the recording first, then drop the file again.')
      if (tr.phase === 'processing') return void toast.info('Parlo is still transcribing. Wait for it to finish or cancel it first.')
      if (tr.phase === 'result' && !(await tr.startNew())) return
      app.tab = 'transcribe'
      await tr.transcribeFile(path, tr.language)
      break
    case 'document':
      if (sp.phase === 'generating') return void toast.info('Parlo is generating speech. Wait for it to finish or cancel it first.')
      app.tab = 'speak'
      await sp.importFile(path)
      break
    default:
      toast.error(`Parlo can’t open “${baseName(path)}”. It handles audio, video, .txt, .md, .docx, .pdf and .epub files.`)
  }
}

const KEYMAP: Array<{ test: (e: KeyboardEvent) => boolean; action: Action }> = [
  { test: (e) => e.metaKey && !e.shiftKey && e.key === ',', action: 'settings' },
  { test: (e) => e.metaKey && !e.shiftKey && e.key === '1', action: 'tab_transcribe' },
  { test: (e) => e.metaKey && !e.shiftKey && e.key === '2', action: 'tab_speak' },
  { test: (e) => e.metaKey && !e.shiftKey && e.key.toLowerCase() === 'o', action: 'open' },
  { test: (e) => e.metaKey && !e.shiftKey && e.key.toLowerCase() === 'r', action: 'record' },
  { test: (e) => e.metaKey && !e.shiftKey && e.key.toLowerCase() === 'p', action: 'pause' },
  { test: (e) => e.metaKey && !e.shiftKey && e.key.toLowerCase() === 'e', action: 'export' },
  { test: (e) => e.metaKey && e.shiftKey && e.key.toLowerCase() === 'c', action: 'copy_transcript' },
  { test: (e) => e.metaKey && e.key === 'Enter', action: 'generate' },
]

/** Wire everything that talks to the outside world. Returns a disposer (used by tests). */
export async function registerInteractions(): Promise<() => void> {
  const b = backend()
  const app = useAppStore()
  const rec = useRecorderStore()
  const off: Unlisten[] = []

  off.push(await b.on('recorder:state', (e) => rec.onState(e)))
  off.push(await b.on('recorder:warning', (e) => rec.onWarning(e.warning)))
  off.push(await b.on('models:state', (e) => app.onModels(e)))
  off.push(await b.on('menu:action', (e) => void runAction(e.action)))
  off.push(await b.on('quit:requested', (e) => (app.quitPrompt = e)))
  off.push(
    await b.onFileDrop(
      (paths) => {
        app.dropHover = false
        if (paths[0]) void handlePath(paths[0])
      },
      (over) => (app.dropHover = over),
    ),
  )

  const onKey = (e: KeyboardEvent) => {
    // In the real app the native menu owns ⌘-shortcuts (accelerators come back as `menu:action`); handling them here
    // as well would fire twice. The mock/browser build has no menu, so it handles them itself.
    if (b.isMock && !e.altKey && !e.ctrlKey) {
      const hit = KEYMAP.find((k) => k.test(e))
      if (hit) {
        e.preventDefault()
        void runAction(hit.action)
        return
      }
    }
    if (e.key === ' ' && !e.metaKey && !e.ctrlKey && !e.altKey && !app.confirm && !app.settingsOpen) {
      const t = e.target as HTMLElement | null
      const typing = t && (t.tagName === 'TEXTAREA' || t.tagName === 'INPUT' || t.tagName === 'SELECT' || t.isContentEditable || t.tagName === 'BUTTON' || t.getAttribute('role') === 'menuitem' || t.getAttribute('role') === 'radio')
      if (!typing) {
        const btn = document.querySelector<HTMLButtonElement>('[data-testid="player-toggle"]')
        if (btn && !btn.disabled) {
          e.preventDefault()
          btn.click()
        }
      }
    }
  }
  // ⌘V of a link on the empty Transcribe screen (not while typing in a field) starts the download
  const onPaste = (e: ClipboardEvent) => {
    if (app.tab !== 'transcribe' || app.confirm || app.settingsOpen) return
    const t = e.target as HTMLElement | null
    if (t && (t.tagName === 'TEXTAREA' || t.tagName === 'INPUT' || t.tagName === 'SELECT' || t.isContentEditable)) return
    const url = parseHttpUrl(e.clipboardData?.getData('text/plain') ?? '')
    if (url && useTranscribeStore().phase === 'empty' && app.settings?.allow_downloads !== false) {
      e.preventDefault()
      void handleLink(url)
    }
  }
  // dragging a link out of a browser's address bar or page onto the window does the same
  const hasLink = (e: DragEvent) => {
    const types = Array.from(e.dataTransfer?.types ?? [])
    return !types.includes('Files') && (types.includes('text/uri-list') || types.includes('text/plain'))
  }
  const canTakeLink = () => app.tab === 'transcribe' && !app.confirm && !app.settingsOpen && useTranscribeStore().phase === 'empty' && app.settings?.allow_downloads !== false
  const onDragOver = (e: DragEvent) => {
    if (hasLink(e) && canTakeLink()) {
      e.preventDefault()
      if (e.dataTransfer) e.dataTransfer.dropEffect = 'copy'
    }
  }
  const onDrop = (e: DragEvent) => {
    if (!hasLink(e) || !canTakeLink()) return
    const link = linkFromDrop(e.dataTransfer?.getData('text/uri-list') ?? '', e.dataTransfer?.getData('text/plain') ?? '')
    if (link) {
      e.preventDefault()
      app.dropHover = false
      void handleLink(link)
    }
  }
  const onVisibility = () => {
    void b.setVisible(document.visibilityState === 'visible')
    if (document.visibilityState === 'visible' && !rec.active) void rec.refreshDevices()
  }
  // no polling: the shown input is re-read whenever the user comes back to the window (e.g. after switching AirPods off)
  const onFocus = () => {
    if (!rec.active) void rec.refreshDevices()
  }
  window.addEventListener('focus', onFocus)
  window.addEventListener('keydown', onKey)
  document.addEventListener('paste', onPaste)
  document.addEventListener('dragover', onDragOver)
  document.addEventListener('drop', onDrop)
  document.addEventListener('visibilitychange', onVisibility)
  void b.setVisible(document.visibilityState === 'visible')

  return () => {
    off.forEach((f) => f())
    window.removeEventListener('keydown', onKey)
    document.removeEventListener('paste', onPaste)
    document.removeEventListener('dragover', onDragOver)
    document.removeEventListener('drop', onDrop)
    document.removeEventListener('visibilitychange', onVisibility)
    window.removeEventListener('focus', onFocus)
  }
}
