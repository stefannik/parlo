import { mount, flushPromises, type VueWrapper } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'
import App from './App.vue'
import { hydrate, registerInteractions } from './lib/bootstrap'
import { setup } from './test/helpers'
import { useAppStore } from './stores/app'

let wrapper: VueWrapper | null = null
let dispose: (() => void) | null = null

async function boot(opts: Parameters<typeof setup>[0] = {}) {
  const ctx = setup(opts)
  dispose = await registerInteractions()
  await hydrate()
  wrapper = mount(App, { global: { plugins: [ctx.pinia] }, attachTo: document.body })
  await flushPromises()
  return { ...ctx, w: wrapper }
}
afterEach(() => {
  wrapper?.unmount()
  dispose?.()
  wrapper = null
  dispose = null
  document.body.innerHTML = ''
  document.documentElement.removeAttribute('data-theme')
})
const tid = (w: VueWrapper, id: string) => w.find(`[data-testid="${id}"]`)

describe('app shell (mock backend)', () => {
  it('opens on Transcribe with Record and the drop zone, and the status chip says idle', async () => {
    const { w } = await boot()
    expect(tid(w, 'transcribe-view').attributes('data-phase')).toBe('empty')
    expect(tid(w, 'record-button').exists()).toBe(true)
    expect(tid(w, 'drop-zone').text()).toContain('Transcribe a file')
    expect(tid(w, 'status-chip').text()).toContain('Idle · no models in memory')
    expect(tid(w, 'tab-transcribe').attributes('aria-checked')).toBe('true')
  })

  it('file → processing (stages, partial text) → result with Copy / Export / New', async () => {
    const { w } = await boot({ timeScale: 0.05 })
    await tid(w, 'choose-file').trigger('click')
    await vi.waitFor(() => expect(tid(w, 'transcribe-view').attributes('data-phase')).toBe('processing'))
    expect(tid(w, 'source-name').text()).toBe('team-meeting.m4a')
    await vi.waitFor(() => expect(tid(w, 'partial').exists()).toBe(true), { timeout: 3000 })
    await vi.waitFor(() => expect(tid(w, 'transcribe-view').attributes('data-phase')).toBe('result'), { timeout: 5000 })
    expect((tid(w, 'transcript-editor').element as HTMLTextAreaElement).value).toContain('Good morning everyone')
    expect(tid(w, 'language-badge').text()).toContain('English')
    expect(tid(w, 'word-count').text()).toMatch(/\d+ words/)
    expect(tid(w, 'copy-button').exists() && tid(w, 'export-button').exists() && tid(w, 'new-button').exists()).toBe(true)
    await tid(w, 'copy-button').trigger('click')
    await flushPromises()
    expect(w.find('[data-testid="toast"]').text()).toContain('copied')
  })

  it('Export menu lists its formats and exports the transcript through a save dialog', async () => {
    const { w, backend } = await boot({ timeScale: 0.02 })
    await tid(w, 'choose-file').trigger('click')
    await vi.waitFor(() => expect(tid(w, 'transcribe-view').attributes('data-phase')).toBe('result'), { timeout: 5000 })
    const spy = vi.spyOn(backend, 'exportTranscript')
    await tid(w, 'export-button').trigger('click')
    await flushPromises()
    expect(w.findAll('[role="menuitem"]').map((m) => m.text())).toEqual(['Text (.txt)', 'Markdown (.md)'])
    await tid(w, 'menu-md').trigger('click')
    await vi.waitFor(() => expect(spy).toHaveBeenCalled())
    expect(spy.mock.calls[0]![1]).toBe('md')
    expect(w.find('[role="menu"]').exists()).toBe(false)
  })

  it('shows the error card with details and Retry', async () => {
    const { w } = await boot({ timeScale: 0.02, failNextStt: true })
    await tid(w, 'choose-file').trigger('click')
    await vi.waitFor(() => expect(tid(w, 'error-card').exists()).toBe(true), { timeout: 3000 })
    expect(tid(w, 'error-card').text()).toContain('read “team-meeting.m4a”')
    expect(tid(w, 'error-details').exists()).toBe(false)
    await tid(w, 'error-card').findAll('button')[1]!.trigger('click')
    expect(tid(w, 'error-details').text()).toContain('unsupported codec')
    await tid(w, 'retry-button').trigger('click')
    await vi.waitFor(() => expect(tid(w, 'transcribe-view').attributes('data-phase')).toBe('result'), { timeout: 5000 })
  })

  it('records: timer + meter + banners, pause, stop & transcribe', async () => {
    const { w, backend } = await boot({ timeScale: 0.02 })
    await tid(w, 'record-button').trigger('click')
    await vi.waitFor(() => expect(tid(w, 'transcribe-view').attributes('data-phase')).toBe('recording'))
    expect(tid(w, 'rec-timer').text()).toMatch(/^\d+:\d\d$/)
    expect(tid(w, 'saved-indicator').text()).toContain('Saved continuously')
    expect(tid(w, 'level-meter').exists()).toBe(true)
    expect(tid(w, 'tab-transcribe').find('.rec-dot').exists()).toBe(true)
    backend.emitWarning({ kind: 'device_changed', from: 'MacBook Pro Microphone', to: "Stefan's AirPods Pro" })
    await flushPromises()
    expect(tid(w, 'rec-banner').text()).toContain('AirPods')
    await tid(w, 'pause-button').trigger('click')
    await vi.waitFor(() => expect(tid(w, 'pause-button').text()).toContain('Resume'))
    await tid(w, 'stop-button').trigger('click')
    await vi.waitFor(() => expect(tid(w, 'transcribe-view').attributes('data-phase')).toBe('processing'))
    await vi.waitFor(() => expect(tid(w, 'transcribe-view').attributes('data-phase')).toBe('result'), { timeout: 5000 })
  })

  it('Discard asks for confirmation in an in-app dialog (never alert())', async () => {
    const alertSpy = vi.spyOn(window, 'alert').mockImplementation(() => {})
    const { w } = await boot()
    await tid(w, 'record-button').trigger('click')
    await vi.waitFor(() => expect(tid(w, 'discard-button').exists()).toBe(true))
    await tid(w, 'discard-button').trigger('click')
    await vi.waitFor(() => expect(tid(w, 'confirm-dialog').exists()).toBe(true))
    await tid(w, 'confirm-cancel').trigger('click')
    expect(tid(w, 'transcribe-view').attributes('data-phase')).toBe('recording')
    await tid(w, 'discard-button').trigger('click')
    await vi.waitFor(() => expect(tid(w, 'confirm-ok').exists()).toBe(true))
    await tid(w, 'confirm-ok').trigger('click')
    await vi.waitFor(() => expect(tid(w, 'transcribe-view').attributes('data-phase')).toBe('empty'))
    expect(alertSpy).not.toHaveBeenCalled()
  })

  it('denied microphone shows the guided fix', async () => {
    const { w } = await boot({ micPermission: 'denied' })
    expect(w.text()).toContain('System Settings')
    expect(tid(w, 'mic-hint').text()).toContain('Microphone access is off')
  })

  it('Speak: type → Generate → player appears while generating → result with Export/Reveal; stale marker', async () => {
    const { w } = await boot({ timeScale: 0.03 })
    await tid(w, 'tab-speak').trigger('click')
    expect(tid(w, 'speak-view').isVisible()).toBe(true)
    expect(tid(w, 'generate').attributes('disabled')).toBeDefined()
    await tid(w, 'speak-editor').setValue('Hello there. '.repeat(60))
    expect(tid(w, 'counts').text()).toMatch(/120 words/)
    expect(tid(w, 'generate').attributes('disabled')).toBeUndefined()
    await tid(w, 'generate').trigger('click')
    await vi.waitFor(() => expect(tid(w, 'player').exists()).toBe(true), { timeout: 3000 })
    await vi.waitFor(() => expect(tid(w, 'speak-view').attributes('data-phase')).toBe('result'), { timeout: 5000 })
    expect(tid(w, 'export-audio').exists() && tid(w, 'reveal').exists()).toBe(true)
    expect(tid(w, 'out-of-date').exists()).toBe(false)
    await tid(w, 'speak-editor').setValue('Something else entirely.')
    expect(tid(w, 'out-of-date').exists()).toBe(true)
  })

  it('dropping an audio file switches to Transcribe; dropping a document switches to Speak', async () => {
    const { w, backend } = await boot({ timeScale: 0.5 })
    const app = useAppStore()
    app.tab = 'speak'
    backend.simulateDrop(['/Users/me/Downloads/interview.mp3'])
    await vi.waitFor(() => expect(app.tab).toBe('transcribe'))
    await vi.waitFor(() => expect(tid(w, 'source-name').text()).toBe('interview.mp3'))
    backend.simulateDrop(['/Users/me/Documents/book.epub'])
    await vi.waitFor(() => expect(app.tab).toBe('speak'))
    await vi.waitFor(() => expect(tid(w, 'source-chip').text()).toContain('book.epub'))
    await tid(w, 'cancel-button').trigger('click').catch(() => {})
  })

  it('an unsupported dropped file produces a toast', async () => {
    const { w, backend } = await boot()
    backend.simulateDrop(['/tmp/archive.zip'])
    await vi.waitFor(() => expect(w.find('[data-testid="toast"]').exists()).toBe(true))
    expect(w.find('[data-testid="toast"]').text()).toContain('archive.zip')
  })

  it('shows the drop overlay while a file is dragged over', async () => {
    const { w, backend } = await boot()
    backend.simulateHover(true)
    await flushPromises()
    expect(tid(w, 'drop-overlay').exists()).toBe(true)
    backend.simulateHover(false)
    await flushPromises()
    await vi.waitFor(() => expect(tid(w, 'drop-overlay').exists()).toBe(false))
  })

  it('keyboard: ⌘2/⌘1 switch tabs, ⌘, opens settings and Esc closes it', async () => {
    const { w } = await boot()
    const app = useAppStore()
    window.dispatchEvent(new KeyboardEvent('keydown', { key: '2', metaKey: true }))
    await flushPromises()
    expect(app.tab).toBe('speak')
    window.dispatchEvent(new KeyboardEvent('keydown', { key: '1', metaKey: true }))
    await flushPromises()
    expect(app.tab).toBe('transcribe')
    window.dispatchEvent(new KeyboardEvent('keydown', { key: ',', metaKey: true }))
    await flushPromises()
    expect(tid(w, 'settings-sheet').exists()).toBe(true)
    tid(w, 'settings-sheet').element.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    await vi.waitFor(() => expect(tid(w, 'settings-sheet').exists()).toBe(false))
  })

  it('menu actions from the native menu are handled (no double handling of key presses)', async () => {
    const { backend } = await boot()
    const app = useAppStore()
    backend.simulateMenu('tab_speak')
    await flushPromises()
    expect(app.tab).toBe('speak')
  })

  it('Settings: appearance applies the theme attribute; sections are reachable', async () => {
    const { w } = await boot()
    await tid(w, 'open-settings').trigger('click')
    await flushPromises()
    const select = w.find('#s-appearance')
    await select.setValue('dark')
    await flushPromises()
    expect(document.documentElement.getAttribute('data-theme')).toBe('dark')
    for (const s of ['recording', 'transcription', 'speech', 'storage', 'about']) {
      await tid(w, `settings-${s}`).trigger('click')
      await flushPromises()
    }
    expect(w.text()).toContain('never connects to the internet')
    await tid(w, 'settings-close').trigger('click')
    await vi.waitFor(() => expect(tid(w, 'settings-sheet').exists()).toBe(false))
  })

  it('Recovery banner offers Transcribe / Save audio / Discard for an interrupted recording', async () => {
    const { w } = await boot({ timeScale: 0.02, recoverables: [{ kind: 'recording', id: 'rec-9', wav_path: '/mock/recordings/rec-9.wav', duration_ms: 754_000, started_at: '2026-10-13T09:00:00Z', saved_on_quit: false }] })
    const banner = tid(w, 'recovery-banner')
    expect(banner.text()).toContain('12:34')
    expect(banner.findAll('button').map((b) => b.text())).toEqual(['Transcribe', 'Save audio…', 'Discard'])
    await banner.findAll('button')[0]!.trigger('click')
    await vi.waitFor(() => expect(tid(w, 'source-name').text()).toBe('Recovered recording'))
    await vi.waitFor(() => expect(tid(w, 'transcribe-view').attributes('data-phase')).toBe('result'), { timeout: 5000 })
    expect(w.find('[data-testid="recovery-banner"]').exists()).toBe(false)
  })

  it('restores the last transcript from the workspace on launch', async () => {
    const { w } = await boot({
      workspace: { transcript: { text: 'Welcome back.', source_name: 'yesterday.m4a', language: 'en', duration_s: 61, words: 2, edited: false, exported_or_copied: true, recording_id: null }, tts: null },
    })
    expect(tid(w, 'transcribe-view').attributes('data-phase')).toBe('result')
    expect((tid(w, 'transcript-editor').element as HTMLTextAreaElement).value).toBe('Welcome back.')
  })

  it('quit request while recording shows the stop-and-save dialog', async () => {
    const { w, backend } = await boot()
    await backend.recordingStart()
    backend.simulateQuit()
    await flushPromises()
    expect(tid(w, 'quit-dialog').text()).toContain('Stop recording and quit?')
  })
})
