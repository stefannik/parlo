import { describe, expect, it, vi } from 'vitest'
import { flush, setup } from '../test/helpers'
import { useAppStore } from './app'
import { useRecorderStore } from './recorder'
import { useTranscribeStore } from './transcribe'

async function finished(tr: ReturnType<typeof useTranscribeStore>) {
  await vi.waitFor(() => expect(['result', 'error', 'idle']).toContain(tr.stage), { timeout: 3000 })
}

describe('transcribe store state machine', () => {
  it('empty → processing → result, streaming paragraphs on the way', async () => {
    setup({ timeScale: 0.05 })
    const tr = useTranscribeStore()
    expect(tr.phase).toBe('empty')
    const p = tr.transcribeFile('/Users/me/talk.m4a')
    await flush()
    expect(tr.phase).toBe('processing')
    await p
    await finished(tr)
    expect(tr.phase).toBe('result')
    expect(tr.transcript?.source_name).toBe('talk.m4a')
    expect(tr.paragraphs.length).toBeGreaterThan(2)
    expect(tr.text).toBe(tr.transcript!.text)
    expect(tr.detectedLanguage?.code).toBe('en')
    expect(tr.edited).toBe(false)
  })

  it('persists the result to the workspace', async () => {
    const { backend } = setup()
    const tr = useTranscribeStore()
    await tr.transcribeFile('/a/talk.m4a')
    await finished(tr)
    await vi.waitFor(async () => expect((await backend.workspaceGet()).transcript?.source_name).toBe('talk.m4a'))
  })

  it('error → retry → result; the source is preserved', async () => {
    const { backend } = setup({ failNextStt: true })
    const tr = useTranscribeStore()
    await tr.transcribeFile('/a/bad.mp3')
    await finished(tr)
    expect(tr.phase).toBe('error')
    expect(tr.error?.code).toBe('decode')
    expect(tr.source?.name).toBe('bad.mp3')
    await tr.retry()
    await finished(tr)
    expect(tr.phase).toBe('result')
    expect(backend.control.options.failNextStt).toBe(false)
  })

  it('cancel returns to the empty state (no error shown)', async () => {
    setup({ timeScale: 0.05 })
    const tr = useTranscribeStore()
    await tr.transcribeFile('/a/long.wav')
    await tr.cancel()
    await finished(tr)
    expect(tr.phase).toBe('empty')
    expect(tr.error).toBeNull()
  })

  it('ignores a second file while one is processing', async () => {
    setup({ timeScale: 0.05 })
    const tr = useTranscribeStore()
    await tr.transcribeFile('/a/one.wav')
    await tr.transcribeFile('/a/two.wav')
    expect(tr.source?.name).toBe('one.wav')
    await tr.cancel()
    await finished(tr)
  })

  it('"New" asks only when the transcript was edited and never copied or exported', async () => {
    setup()
    const app = useAppStore()
    const tr = useTranscribeStore()
    await tr.transcribeFile('/a/talk.m4a')
    await finished(tr)

    // untouched → no confirmation
    const askSpy = vi.spyOn(app, 'ask')
    expect(await tr.startNew()).toBe(true)
    expect(askSpy).not.toHaveBeenCalled()

    // edited → confirmation; declining keeps the transcript
    await tr.transcribeFile('/a/talk.m4a')
    await finished(tr)
    tr.edit('my changes')
    askSpy.mockResolvedValueOnce(false)
    expect(await tr.startNew()).toBe(false)
    expect(tr.phase).toBe('result')
    expect(tr.text).toBe('my changes')

    // edited then copied → no confirmation
    await tr.copy()
    expect(tr.exportedOrCopied).toBe(true)
    askSpy.mockClear()
    expect(await tr.startNew()).toBe(true)
    expect(askSpy).not.toHaveBeenCalled()
    expect(tr.phase).toBe('empty')
  })

  it('editing again after copying re-arms the confirmation', async () => {
    setup()
    const tr = useTranscribeStore()
    await tr.transcribeFile('/a/talk.m4a')
    await finished(tr)
    tr.edit('one')
    await tr.copy()
    tr.edit('two')
    expect(tr.exportedOrCopied).toBe(false)
  })

  it('re-transcribing in another language asks first when edited', async () => {
    setup()
    const app = useAppStore()
    const tr = useTranscribeStore()
    await tr.transcribeFile('/a/talk.m4a')
    await finished(tr)
    tr.edit('edited')
    const ask = vi.spyOn(app, 'ask').mockResolvedValueOnce(false)
    await tr.retranscribe('de')
    expect(ask).toHaveBeenCalledOnce()
    expect(tr.phase).toBe('result')
    ask.mockResolvedValueOnce(true)
    await tr.retranscribe('de')
    await finished(tr)
    expect(tr.language).toBe('de')
    expect(tr.detectedLanguage?.code).toBe('de')
    expect(tr.edited).toBe(false)
  })

  it('exports ask for a destination and mark the transcript as exported', async () => {
    const { backend } = setup()
    const tr = useTranscribeStore()
    await tr.transcribeFile('/a/talk.m4a')
    await finished(tr)
    const save = vi.spyOn(backend, 'saveFile')
    const exp = vi.spyOn(backend, 'exportTranscript')
    await tr.exportAs('md')
    expect(save).toHaveBeenCalledWith('talk.md', ['md'])
    expect(exp).toHaveBeenCalled()
    expect(tr.exportedOrCopied).toBe(true)
    // cancelling the save dialog exports nothing
    exp.mockClear()
    save.mockResolvedValueOnce(null)
    await tr.exportAs('txt')
    expect(exp).not.toHaveBeenCalled()
  })

  it('recording state comes from the recorder (also when the window was reopened mid-recording)', async () => {
    const { backend } = setup()
    const rec = useRecorderStore()
    const tr = useTranscribeStore()
    await backend.on('recorder:state', (e) => rec.onState(e))
    await tr.startRecording()
    await flush()
    expect(tr.phase).toBe('recording')
    await tr.stopAndTranscribe()
    expect(tr.phase).toBe('processing')
    expect(tr.source?.recordingId).toMatch(/^rec-/)
    await finished(tr)
    expect(tr.phase).toBe('result')
    expect(tr.transcript?.recording_id).toBe(tr.source?.recordingId)
  })

  it('denied microphone permission leaves the empty state with a hint, never throws', async () => {
    const { backend } = setup({ micPermission: 'denied' })
    const rec = useRecorderStore()
    const tr = useTranscribeStore()
    await backend.on('recorder:state', (e) => rec.onState(e))
    await rec.refreshPermission()
    await tr.startRecording()
    expect(tr.phase).toBe('empty')
    expect(rec.permission).toBe('denied')
    expect(rec.lastError).toMatch(/Microphone/i)
  })

  it('discarding a recording asks for confirmation', async () => {
    const { backend } = setup()
    const app = useAppStore()
    const rec = useRecorderStore()
    const tr = useTranscribeStore()
    await backend.on('recorder:state', (e) => rec.onState(e))
    await tr.startRecording()
    const ask = vi.spyOn(app, 'ask').mockResolvedValueOnce(false)
    await tr.discardRecording()
    expect(rec.active).toBe(true)
    ask.mockResolvedValueOnce(true)
    await tr.discardRecording()
    await flush()
    expect(rec.active).toBe(false)
    expect(tr.phase).toBe('empty')
  })

  it('restores a persisted transcript as a result', async () => {
    setup()
    const tr = useTranscribeStore()
    tr.restore({ text: 'restored text here', source_name: 'old.m4a', language: 'fr', duration_s: 10, words: 3, edited: true, exported_or_copied: false, recording_id: null })
    expect(tr.phase).toBe('result')
    expect(tr.edited).toBe(true)
    expect(tr.words).toBe(3)
  })
})
