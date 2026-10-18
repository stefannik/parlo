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
    expect(w.text()).toContain('works offline')
    expect(w.text()).toContain('a download you start from a link')
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
      workspace: { transcript: { text: 'Welcome back.', source_name: 'yesterday.m4a', language: 'en', duration_s: 61, words: 2, edited: false, exported_or_copied: true, recording_id: null, source_url: null, download_id: null, media_path: null }, tts: null, pending_url: null },
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

  // ------------------------------------------------------------------------------------------ Transcribe from URL
  describe('Transcribe from a link', () => {
    const paste = (text: string, target: EventTarget = document.body) => {
      const ev = new Event('paste', { bubbles: true, cancelable: true }) as Event & { clipboardData?: unknown }
      ev.clipboardData = { getData: (t: string) => (t === 'text/plain' ? text : '') }
      target.dispatchEvent(ev)
      return ev
    }
    const drop = (data: Record<string, string>) => {
      const ev = new Event('drop', { bubbles: true, cancelable: true }) as Event & { dataTransfer?: unknown }
      ev.dataTransfer = { types: Object.keys(data), getData: (t: string) => data[t] ?? '' }
      document.body.dispatchEvent(ev)
      return ev
    }

    it('the empty screen has a third card with a link field and a Transcribe button', async () => {
      const { w } = await boot()
      expect(tid(w, 'url-card').text()).toContain('Transcribe a link')
      expect(tid(w, 'url-input').attributes('placeholder')).toBe('https://…')
      expect(tid(w, 'record-button').exists() && tid(w, 'drop-zone').exists()).toBe(true)
    })

    it('typing a link and pressing Transcribe shows the Downloading stage (title, waiting, bytes) and then the result', async () => {
      const { w, backend } = await boot({ timeScale: 0.03 })
      backend.control.hold('stt:waiting')
      await tid(w, 'url-input').setValue('https://www.youtube.com/watch?v=abc')
      await tid(w, 'url-submit').trigger('click')
      await vi.waitFor(() => expect(tid(w, 'transcribe-view').attributes('data-phase')).toBe('processing'))
      await vi.waitFor(() => expect(tid(w, 'source-name').text()).toBe('Local-first software: an interview with the authors'))
      expect(w.find('[data-stage="downloading"]').classes()).toContain('current')
      await vi.waitFor(() => expect(tid(w, 'eta').text()).toMatch(/^Waiting \d+ s…$/))
      expect(tid(w, 'download-waiting').exists()).toBe(true)
      expect(tid(w, 'proc-subtitle').text()).toContain('1 h 02 min') // the video's length, known before any bytes
      backend.control.release()
      await vi.waitFor(() => expect(tid(w, 'eta').text()).toMatch(/%.*MB of 36\.6 MB.*MB\/s.*left/), { timeout: 4000 })
      await vi.waitFor(() => expect(tid(w, 'transcribe-view').attributes('data-phase')).toBe('result'), { timeout: 8000 })
      expect(tid(w, 'source-host').text()).toBe('youtube.com')
      expect(tid(w, 'save-audio-button').exists()).toBe(true)
    })

    it('Enter in the field starts it; a bad link shows an inline message and starts nothing', async () => {
      const { w, backend } = await boot({ timeScale: 0.02 })
      const spy = vi.spyOn(backend, 'transcribe')
      await tid(w, 'url-input').setValue('hello world')
      await tid(w, 'url-submit').trigger('click')
      expect(tid(w, 'url-error').text()).toContain('single web link')
      expect(spy).not.toHaveBeenCalled()
      await tid(w, 'url-input').setValue('ftp://example.com/a.mp3')
      await tid(w, 'url-input').trigger('keydown', { key: 'Enter' })
      await tid(w, 'url-input').trigger('submit')
      await tid(w, 'url-submit').trigger('click')
      expect(tid(w, 'url-error').text()).toContain('Only http')
      await tid(w, 'url-input').setValue('https://example.com/ep.mp3')
      expect(tid(w, 'url-error').exists()).toBe(false)
      await tid(w, 'url-input').trigger('submit')
      await vi.waitFor(() => expect(spy).toHaveBeenCalledOnce())
      await vi.waitFor(() => expect(tid(w, 'transcribe-view').attributes('data-phase')).toBe('result'), { timeout: 8000 })
    })

    it('⌘V of a link on the empty screen starts it; pasting into a text field or non-links does not', async () => {
      const { w, backend } = await boot({ timeScale: 0.02 })
      const spy = vi.spyOn(backend, 'transcribe')
      paste('just some words')
      paste('https://example.com/ep.mp3', tid(w, 'url-input').element)
      await flushPromises()
      expect(spy).not.toHaveBeenCalled()
      const ev = paste('  https://example.com/ep.mp3\n')
      expect(ev.defaultPrevented).toBe(true)
      await vi.waitFor(() => expect(spy).toHaveBeenCalledOnce())
      expect(spy.mock.calls[0]![0].source).toEqual({ kind: 'url', url: 'https://example.com/ep.mp3' })
      await vi.waitFor(() => expect(tid(w, 'transcribe-view').attributes('data-phase')).toBe('result'), { timeout: 8000 })
      // a result is on screen now: another paste is ignored
      paste('https://example.com/other.mp3')
      await flushPromises()
      expect(spy).toHaveBeenCalledOnce()
    })

    it('a link dragged from a browser starts it (uri-list or plain text); dragged files are left to the file drop', async () => {
      const { w, backend } = await boot({ timeScale: 0.02 })
      const spy = vi.spyOn(backend, 'transcribe')
      drop({ Files: '' })
      drop({ 'text/plain': 'not a link' })
      await flushPromises()
      expect(spy).not.toHaveBeenCalled()
      const ev = drop({ 'text/uri-list': '# from Safari\r\nhttps://example.com/ep.m4a\r\n', 'text/plain': 'https://example.com/ep.m4a' })
      expect(ev.defaultPrevented).toBe(true)
      await vi.waitFor(() => expect(spy).toHaveBeenCalledOnce())
      await vi.waitFor(() => expect(tid(w, 'transcribe-view').attributes('data-phase')).toBe('result'), { timeout: 8000 })
    })

    it('Cancel while downloading returns to the empty screen with the link still in the field', async () => {
      const { w } = await boot({ timeScale: 0.05 })
      await tid(w, 'url-input').setValue('https://example.com/ep.mp3')
      await tid(w, 'url-submit').trigger('click')
      await vi.waitFor(() => expect(w.find('[data-stage="downloading"]').classes()).toContain('current'))
      await tid(w, 'cancel-button').trigger('click')
      await vi.waitFor(() => expect(tid(w, 'transcribe-view').attributes('data-phase')).toBe('empty'), { timeout: 4000 })
      expect((tid(w, 'url-input').element as HTMLInputElement).value).toBe('https://example.com/ep.mp3')
      expect(w.find('[data-testid="error-card"]').exists()).toBe(false)
    })

    it('a private video shows the cookies hint and a button that opens Settings → Downloads', async () => {
      const { w } = await boot({ timeScale: 0.02 })
      await tid(w, 'url-input').setValue('https://www.youtube.com/private')
      await tid(w, 'url-submit').trigger('click')
      await vi.waitFor(() => expect(tid(w, 'error-card').exists()).toBe(true), { timeout: 4000 })
      expect(tid(w, 'error-card').text()).toContain('Use browser cookies')
      expect(tid(w, 'error-link').text()).toContain('https://www.youtube.com/private')
      await tid(w, 'open-download-settings').trigger('click')
      await flushPromises()
      expect(tid(w, 'downloads-section').exists()).toBe(true)
      await tid(w, 'settings-close').trigger('click')
      await tid(w, 'retry-button').trigger('click') // Retry downloads the same link again
      await vi.waitFor(() => expect(tid(w, 'error-card').exists()).toBe(true), { timeout: 4000 })
    })

    it('an interrupted download refills the link field on launch', async () => {
      const { w } = await boot({ workspace: { transcript: null, tts: null, pending_url: 'https://example.com/half-done.mp3' } })
      expect((tid(w, 'url-input').element as HTMLInputElement).value).toBe('https://example.com/half-done.mp3')
    })

    it('Settings → Downloads: downloader, browsers, toggles; switching downloads off hides the card and refuses links', async () => {
      const { w, backend } = await boot({ timeScale: 0.02 })
      const app = useAppStore()
      app.settingsOpen = true
      await flushPromises()
      await tid(w, 'settings-downloads').trigger('click')
      await vi.waitFor(() => expect(tid(w, 'downloader-list').exists()).toBe(true))
      expect(tid(w, 'tool-bundled').text()).toContain('2026.08.19')
      expect(tid(w, 'tool-bundled').text()).toContain('in use')
      expect(tid(w, 'tool-system').text()).toContain('fallback')
      expect(tid(w, 'tool-system').text()).toContain('No Chrome impersonation')
      expect(tid(w, 'deno-status').text()).toContain('/opt/homebrew/bin/deno')
      // cookies: off by default, the picker lists only installed browsers
      expect((tid(w, 'use-cookies').element as HTMLInputElement).checked).toBe(false)
      expect(tid(w, 'cookies-browser').exists()).toBe(false)
      await tid(w, 'use-cookies').setValue(true)
      await vi.waitFor(() => expect(tid(w, 'cookies-browser').exists()).toBe(true))
      expect(tid(w, 'cookies-browser').findAll('option').map((o) => o.text())).toEqual(['Google Chrome', 'Firefox', 'Safari'])
      expect(app.settings).toMatchObject({ use_browser_cookies: true, cookies_browser: 'chrome' })
      await tid(w, 'cookies-browser').setValue('firefox')
      expect(app.settings?.cookies_browser).toBe('firefox')
      await tid(w, 'impersonate').setValue(false)
      await tid(w, 'rate-limit').setValue(false)
      expect(app.settings).toMatchObject({ impersonate: false, rate_limit_sleep: false })
      // master switch
      await tid(w, 'allow-downloads').setValue(false)
      await flushPromises()
      expect(app.settings?.allow_downloads).toBe(false)
      expect(tid(w, 'use-cookies').exists()).toBe(false)
      await tid(w, 'settings-close').trigger('click')
      expect(tid(w, 'url-card').exists()).toBe(false)
      expect(tid(w, 'record-button').exists() && tid(w, 'drop-zone').exists()).toBe(true)
      const spy = vi.spyOn(backend, 'transcribe')
      paste('https://example.com/ep.mp3')
      await flushPromises()
      expect(spy).not.toHaveBeenCalled()
    })
  })
})
