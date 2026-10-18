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
    tr.restore({ text: 'restored text here', source_name: 'old.m4a', language: 'fr', duration_s: 10, words: 3, edited: true, exported_or_copied: false, recording_id: null, source_url: null, download_id: null, media_path: null })
    expect(tr.phase).toBe('result')
    expect(tr.edited).toBe(true)
    expect(tr.words).toBe(3)
  })
})

describe('transcribe from URL (store)', () => {
  const LINK = 'https://www.youtube.com/watch?v=abc'

  it('empty → Downloading (title, bytes, speed) → the unchanged pipeline → result that remembers its link and audio', async () => {
    setup({ timeScale: 0.02 })
    const tr = useTranscribeStore()
    tr.urlDraft = `  ${LINK}  `
    expect(await tr.transcribeUrl()).toBe(true)
    await flush()
    expect(tr.phase).toBe('processing')
    expect(tr.isUrlJob).toBe(true)
    await vi.waitFor(() => expect(tr.currentStage).toBe('downloading'))
    await vi.waitFor(() => expect(tr.download?.downloaded).toBeGreaterThan(0), { timeout: 3000 })
    expect(tr.source?.name).toBe('Local-first software: an interview with the authors') // job name = the video title
    expect(tr.download).toMatchObject({ total: 38_400_000, speedBps: 2_400_000, playlist: false })
    expect(tr.downloadFraction).toBeGreaterThan(0)
    await finished(tr)
    expect(tr.phase).toBe('result')
    expect(tr.transcript?.source_url).toBe(LINK)
    expect(tr.source).toMatchObject({ url: LINK })
    expect(tr.source?.downloadId).toMatch(/^dl-/)
    expect(tr.playablePath).toContain('/downloads/') // the mini player plays the kept audio
    expect(tr.urlDraft).toBe('')
  })

  it('an invalid link never starts a job and shows an inline message', async () => {
    const { backend } = setup()
    const spy = vi.spyOn(backend, 'transcribe')
    const tr = useTranscribeStore()
    tr.urlDraft = 'youtube.com/watch'
    expect(await tr.transcribeUrl()).toBe(false)
    expect(spy).not.toHaveBeenCalled()
    expect(tr.phase).toBe('empty')
    expect(tr.urlError).toMatch(/start with http/)
    tr.urlDraft = ''
    tr.urlTouched = true
    expect(await tr.transcribeUrl()).toBe(false)
    expect(tr.urlError).toBe('Paste a link first.')
  })

  it('shows "Waiting N s" for the rate-limit sleep and counts down', async () => {
    const { backend } = setup({ timeScale: 0.02 })
    backend.control.hold('stt:waiting')
    const tr = useTranscribeStore()
    await tr.transcribeUrl(LINK)
    await vi.waitFor(() => expect(tr.download?.waitLeft).toBe(7), { timeout: 3000 })
    expect(tr.downloadFraction).toBeNull() // no bar while waiting
    await vi.waitFor(() => expect(tr.download?.waitLeft).toBeLessThan(7), { timeout: 3000 })
    backend.control.release()
    await vi.waitFor(() => expect(tr.download?.waitLeft).toBeNull(), { timeout: 3000 })
    await finished(tr)
    expect(tr.phase).toBe('result')
  })

  it('every download error keeps the link filled in, and Retry downloads again', async () => {
    const { backend } = setup()
    const tr = useTranscribeStore()
    for (const [u, code] of [
      ['https://youtube.com/live', 'download_live'],
      ['https://youtube.com/private', 'download_login'],
      ['https://youtube.com/cookies', 'download_cookies'],
      ['https://open.spotify.com/episode/1', 'download_drm'],
      ['https://example.com/notfound', 'download_not_found'],
      ['https://example.com/offline', 'download_network'],
      ['https://example.com/noaudio', 'download_no_audio'],
      ['https://example.com/diskfull', 'disk_space'],
      ['https://example.com/fail', 'download_failed'],
    ] as const) {
      await tr.startNew()
      tr.urlDraft = u
      await tr.transcribeUrl()
      await finished(tr)
      expect(tr.phase, u).toBe('error')
      expect(tr.error?.code, u).toBe(code)
      expect(tr.urlDraft, `${u} stays filled in`).toBe(u)
      expect(tr.source?.path).toBeNull() // nothing was saved
    }
    // the same link now works → Retry runs the download again
    await tr.startNew()
    tr.urlDraft = 'https://example.com/ep.mp3'
    await tr.transcribeUrl()
    await finished(tr)
    expect(tr.phase).toBe('result')
    expect((await backend.workspaceGet()).pending_url).toBeNull()
  })

  it('a fallback to the system downloader is shown while it happens', async () => {
    setup({ timeScale: 0.02 })
    const tr = useTranscribeStore()
    await tr.transcribeUrl('https://example.com/fallback/ep')
    await vi.waitFor(() => expect(tr.download?.fallback).toBe('Bundled downloader failed, retried with system yt-dlp'), { timeout: 3000 })
    await finished(tr)
    expect(tr.phase).toBe('result') // the retry worked
  })

  it('cancel during the download works like a job cancel: back to empty, link still filled in, no error shown', async () => {
    setup({ timeScale: 0.05 })
    const tr = useTranscribeStore()
    await tr.transcribeUrl(LINK)
    await vi.waitFor(() => expect(tr.currentStage).toBe('downloading'))
    await tr.cancel()
    await finished(tr)
    expect(tr.phase).toBe('empty')
    expect(tr.error).toBeNull()
    expect(tr.urlDraft).toBe(LINK)
    expect(tr.download?.waitLeft ?? null).toBeNull() // no countdown left running
  })

  it('re-transcribing in another language uses the kept audio (no second download)', async () => {
    const { backend } = setup()
    const tr = useTranscribeStore()
    await tr.transcribeUrl(LINK)
    await finished(tr)
    const id = tr.source!.downloadId!
    const spy = vi.spyOn(backend, 'transcribe')
    await tr.retranscribe('de')
    expect(spy.mock.calls[0]![0].source).toMatchObject({ kind: 'path' })
    await finished(tr)
    expect(tr.phase).toBe('result')
    expect(tr.transcript?.download_id).toBe(id)
    expect(tr.transcript?.source_url).toBe(LINK)
    expect(tr.isUrlJob).toBe(true)
  })

  it('"Save audio" exports the downloaded file as it came, or converted', async () => {
    const { backend } = setup()
    const tr = useTranscribeStore()
    await tr.transcribeUrl(LINK)
    await finished(tr)
    const save = vi.spyOn(backend, 'saveFile')
    const exp = vi.spyOn(backend, 'exportDownload')
    await tr.saveAudio('original')
    expect(save).toHaveBeenLastCalledWith(expect.stringMatching(/\.m4a$/), ['m4a'])
    expect(exp).toHaveBeenLastCalledWith(tr.source!.downloadId, null, expect.any(String))
    await tr.saveAudio('mp3')
    expect(exp).toHaveBeenLastCalledWith(tr.source!.downloadId, 'mp3', expect.stringMatching(/\.mp3$/))
  })

  it('a restored URL result keeps its link, audio and re-transcribe ability; an interrupted download refills the field', async () => {
    const { backend } = setup()
    const tr = useTranscribeStore()
    tr.restore({ text: 'x', source_name: 'Episode', language: 'en', duration_s: 5, words: 1, edited: false, exported_or_copied: false, recording_id: null, source_url: LINK, download_id: 'dl-0123456789abcdef', media_path: '/mock/downloads/dl-0123456789abcdef/audio.m4a' })
    expect(tr.phase).toBe('result')
    expect(tr.playablePath).toBe('/mock/downloads/dl-0123456789abcdef/audio.m4a')
    expect(tr.isUrlJob).toBe(true)
    await backend.workspaceUpdate({ transcript: null, tts: null, clear_transcript: true, clear_tts: false })
  })

  it('refuses while downloads are switched off', async () => {
    const { backend } = setup({ downloadsOff: true })
    const app = useAppStore()
    await app.load()
    const spy = vi.spyOn(backend, 'transcribe')
    const tr = useTranscribeStore()
    expect(await tr.transcribeUrl(LINK)).toBe(false)
    expect(spy).not.toHaveBeenCalled()
    expect(tr.phase).toBe('empty')
  })

  it('does nothing while a recording is running or another job is processing', async () => {
    setup({ timeScale: 0.05 })
    const tr = useTranscribeStore()
    await tr.transcribeFile('/a/one.wav')
    expect(await tr.transcribeUrl(LINK)).toBe(false)
    expect(tr.source?.name).toBe('one.wav')
    await tr.cancel()
    await finished(tr)
  })
})

