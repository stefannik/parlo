// The mock must honour the same contract as the real backend (BRIEF §14.1 "IPC mock contract tests"): same event
// ordering, same error codes, same state transitions. These assertions are what the UI relies on.
import { describe, expect, it } from 'vitest'
import type { RecorderStateEvent, SttEventDto, TtsEventDto } from '../bindings'
import { settingsPatch } from '../lib/settings'
import { flush, setup } from '../test/helpers'
import { MockBackend } from './mock'

async function collect<T extends { type: string }>(start: (cb: (e: T) => void) => Promise<unknown>) {
  const events: T[] = []
  let finish: () => void
  const done = new Promise<void>((r) => (finish = r))
  await start((e) => {
    events.push(e)
    if (e.type === 'done' || e.type === 'error') finish()
  })
  await done
  return events
}

describe('mock backend contract', () => {
  it('recording: idle → recording → paused → recording → idle, with elapsed time that does not advance while paused', async () => {
    const { backend } = setup({ timeScale: 1 })
    const states: RecorderStateEvent[] = []
    await backend.on('recorder:state', (e) => states.push(e))
    const info = await backend.recordingStart()
    expect(info.id).toMatch(/^rec-/)
    expect(states.at(-1)?.state).toBe('recording')
    await backend.recordingPause()
    const pausedAt = states.at(-1)!
    expect(pausedAt.state).toBe('paused')
    await new Promise((r) => setTimeout(r, 120))
    expect(states.at(-1)!.elapsed_ms).toBe(pausedAt.elapsed_ms)
    await backend.recordingResume()
    expect(states.at(-1)?.state).toBe('recording')
    const stopped = await backend.recordingStop()
    expect(stopped.wav_path).toMatch(/\.wav$/)
    expect(states.at(-1)?.state).toBe('idle')
    await expect(backend.recordingStop()).rejects.toMatchObject({ code: 'invalid' })
  })

  it('recording refuses to start without microphone permission', async () => {
    const { backend } = setup({ micPermission: 'denied' })
    await expect(backend.recordingStart()).rejects.toMatchObject({ code: 'mic_permission' })
    expect(await backend.micStatus()).toBe('denied')
  })

  it('a second recording cannot start while one runs', async () => {
    const { backend } = setup()
    await backend.recordingStart()
    await expect(backend.recordingStart()).rejects.toMatchObject({ code: 'busy' })
    await backend.recordingDiscard('x')
  })

  it('transcribe: stage → … → paragraphs → done, in order, ending with exactly one terminal event', async () => {
    const { backend } = setup()
    const ev = await collect<SttEventDto>((cb) => backend.transcribe({ source: { kind: 'path', path: '/a/talk.m4a' }, language: 'auto' }, cb))
    const types = ev.map((e) => e.type)
    expect(types[0]).toBe('stage')
    expect(types.filter((t) => t === 'done' || t === 'error')).toEqual(['done'])
    expect(types.at(-1)).toBe('done')
    expect(types.indexOf('language')).toBeGreaterThan(-1)
    const stages = ev.filter((e): e is Extract<SttEventDto, { type: 'stage' }> => e.type === 'stage').map((e) => e.stage)
    expect(stages).toEqual(['preparing', 'detecting_speech', 'detecting_language', 'loading_model', 'transcribing'])
    const paras = ev.filter((e): e is Extract<SttEventDto, { type: 'paragraph' }> => e.type === 'paragraph')
    expect(paras.map((p) => p.index)).toEqual(paras.map((_, i) => i))
    const done = ev.at(-1) as Extract<SttEventDto, { type: 'done' }>
    expect(done.transcript.text).toContain(paras[0]!.text)
    expect(done.transcript.source_name).toBe('talk.m4a')
    expect(done.transcript.words).toBeGreaterThan(20)
  })

  it('transcribe with a fixed language skips language detection', async () => {
    const { backend } = setup()
    const ev = await collect<SttEventDto>((cb) => backend.transcribe({ source: { kind: 'path', path: '/a/x.mp3' }, language: 'de' }, cb))
    const stages = ev.filter((e) => e.type === 'stage').map((e) => (e as { stage: string }).stage)
    expect(stages).not.toContain('detecting_language')
    expect((ev.find((e) => e.type === 'language') as { code: string }).code).toBe('de')
  })

  it('transcribe can be cancelled and reports code "cancelled"', async () => {
    const { backend } = setup({ timeScale: 0.05 })
    const events: SttEventDto[] = []
    const id = await backend.transcribe({ source: { kind: 'path', path: '/a/x.mp3' }, language: 'auto' }, (e) => events.push(e))
    await backend.jobCancel(id)
    await new Promise((r) => setTimeout(r, 400))
    const last = events.at(-1)!
    expect(last.type).toBe('error')
    expect((last as Extract<SttEventDto, { type: 'error' }>).error.code).toBe('cancelled')
  })

  it('failNextStt yields a decode error once', async () => {
    const { backend } = setup({ failNextStt: true })
    const first = await collect<SttEventDto>((cb) => backend.transcribe({ source: { kind: 'path', path: '/a/bad.mp3' }, language: 'auto' }, cb))
    expect(first.at(-1)).toMatchObject({ type: 'error', error: { code: 'decode' } })
    const second = await collect<SttEventDto>((cb) => backend.transcribe({ source: { kind: 'path', path: '/a/bad.mp3' }, language: 'auto' }, cb))
    expect(second.at(-1)?.type).toBe('done')
  })

  it('tts: segment_ready precedes done so playback can start early; progress is monotonic', async () => {
    const { backend } = setup()
    const text = 'word '.repeat(400)
    const ev = await collect<TtsEventDto>((cb) => backend.tts({ source: { kind: 'text', text }, voice: 'george', quality: 'standard' }, cb))
    const types = ev.map((e) => e.type)
    expect(types.indexOf('segment_ready')).toBeLessThan(types.indexOf('done'))
    const prog = ev.filter((e): e is Extract<TtsEventDto, { type: 'progress' }> => e.type === 'progress').map((e) => e.done_chars)
    expect([...prog].sort((a, b) => a - b)).toEqual(prog)
    const done = ev.at(-1) as Extract<TtsEventDto, { type: 'done' }>
    expect(done.segments.length).toBe(ev.filter((e) => e.type === 'segment_ready').length)
    expect(done.duration_ms).toBeGreaterThan(0)
    expect(done.output_id).toMatch(/^out-/)
  })

  it('tts output id is content-addressed (same text + voice → same id)', async () => {
    const { backend } = setup()
    const run = async (voice: string) => ((await collect<TtsEventDto>((cb) => backend.tts({ source: { kind: 'text', text: 'Hello there.' }, voice, quality: 'standard' }, cb))).at(-1) as Extract<TtsEventDto, { type: 'done' }>).output_id
    expect(await run('george')).toBe(await run('george'))
    expect(await run('george')).not.toBe(await run('alba'))
  })

  it('voices: 21 English voices (George among them) + the Spanish voice, each tagged with its language, no duplicates', async () => {
    const { backend } = setup()
    const v = await backend.voices()
    expect(v).toHaveLength(22)
    expect(new Set(v.map((x) => x.id)).size).toBe(22)
    expect(v.filter((x) => x.language === 'en')).toHaveLength(21)
    expect(v.filter((x) => x.language === 'es').map((x) => x.id)).toEqual(['lola'])
    expect(v.map((x) => x.id)).toContain('george')
    // the owner's favourites from the old ten are still there
    for (const id of ['alba', 'anna', 'charles', 'cosette', 'jane', 'mary', 'michael', 'paul', 'vera']) expect(v.map((x) => x.id)).toContain(id)
  })

  it('import returns text + metadata and importFull returns the text', async () => {
    const { backend } = setup()
    const r = await backend.importText('/docs/notes.pdf')
    expect(r.file_name).toBe('notes.pdf')
    expect(r.pages).toBe(12)
    expect(r.warnings.length).toBeGreaterThan(0)
    expect(await backend.importFull(r.import_id)).toBe(r.text)
  })

  it('settings patch merges, and "" resets the input device to the system default', async () => {
    const { backend } = setup()
    let s = await backend.settingsUpdate(settingsPatch({ appearance: 'dark', input_device: 'airpods' }))
    expect(s.appearance).toBe('dark')
    expect(s.input_device).toBe('airpods')
    expect(s.default_voice).toBe('george')
    s = await backend.settingsUpdate(settingsPatch({ input_device: '' }))
    expect(s.input_device).toBeNull()
  })

  it('workspace round-trips and clears', async () => {
    const { backend } = setup()
    await backend.workspaceUpdate({ transcript: { text: 'hi', source_name: 'a.m4a', language: 'en', duration_s: 3, words: 1, edited: false, exported_or_copied: false, recording_id: null, source_url: null, download_id: null, media_path: null }, tts: null, clear_transcript: false, clear_tts: false })
    expect((await backend.workspaceGet()).transcript?.text).toBe('hi')
    await backend.workspaceUpdate({ transcript: null, tts: null, clear_transcript: true, clear_tts: false })
    expect((await backend.workspaceGet()).transcript).toBeNull()
  })

  it('recoverables can be discarded', async () => {
    const { backend } = setup({ recoverables: [{ kind: 'recording', id: 'r1', wav_path: '/x.wav', duration_ms: 1000, started_at: new Date().toISOString(), saved_on_quit: false }] })
    expect(await backend.recoverables()).toHaveLength(1)
    await backend.recoverableDiscard('recording', 'r1')
    expect(await backend.recoverables()).toHaveLength(0)
  })

  it('level events flow only while recording and visible', async () => {
    const { backend } = setup({ timeScale: 1 })
    const levels: number[] = []
    await backend.on('recorder:state', (e) => e.state === 'recording' && levels.push(e.peak))
    await backend.setVisible(false)
    await backend.recordingStart()
    const before = levels.length
    await new Promise((r) => setTimeout(r, 350))
    expect(levels.length).toBe(before)
    await backend.setVisible(true)
    await new Promise((r) => setTimeout(r, 350))
    expect(levels.length).toBeGreaterThan(before)
    await backend.recordingStop()
    const after = levels.length
    await new Promise((r) => setTimeout(r, 250))
    expect(levels.length).toBe(after)
  })

  it('is flagged as the mock', async () => {
    const { backend } = setup()
    expect(backend).toBeInstanceOf(MockBackend)
    expect(backend.isMock).toBe(true)
    await flush()
  })

  // ------------------------------------------------------------------------------------------ Transcribe from URL
  describe('transcribe from URL', () => {
    const url = (u: string) => ({ source: { kind: 'url' as const, url: u }, language: 'auto' })
    const types = (ev: SttEventDto[]) => ev.map((e) => e.type)
    const err = (ev: SttEventDto[]) => (ev.at(-1) as Extract<SttEventDto, { type: 'error' }>).error

    it('Downloading stage first, then the unchanged pipeline: info → waiting → progress → downloaded → preparing … done', async () => {
      const { backend } = setup()
      const ev = await collect<SttEventDto>((cb) => backend.transcribe(url('https://www.youtube.com/watch?v=abc'), cb))
      const t = types(ev)
      expect(ev[0]).toEqual({ type: 'stage', stage: 'downloading' })
      expect(t.indexOf('download_info')).toBeLessThan(t.indexOf('download_waiting'))
      expect(t.indexOf('download_waiting')).toBeLessThan(t.indexOf('download_progress'))
      expect(t.indexOf('download_progress')).toBeLessThan(t.indexOf('downloaded'))
      expect(t.indexOf('downloaded')).toBeLessThan(t.indexOf('paragraph'))
      const stages = ev.filter((e): e is Extract<SttEventDto, { type: 'stage' }> => e.type === 'stage').map((e) => e.stage)
      expect(stages).toEqual(['downloading', 'preparing', 'detecting_speech', 'detecting_language', 'loading_model', 'transcribing'])
      expect(t.filter((x) => x === 'done' || x === 'error')).toEqual(['done'])
      const prog = ev.filter((e): e is Extract<SttEventDto, { type: 'download_progress' }> => e.type === 'download_progress').map((e) => e.downloaded_bytes)
      expect([...prog].sort((a, b) => a - b)).toEqual(prog)
      const done = ev.at(-1) as Extract<SttEventDto, { type: 'done' }>
      const info = ev.find((e) => e.type === 'download_info') as Extract<SttEventDto, { type: 'download_info' }>
      expect(done.transcript.source_name).toBe(info.title)
      expect(done.transcript.source_url).toBe('https://www.youtube.com/watch?v=abc')
      expect(done.transcript.download_id).toMatch(/^dl-[0-9a-f]{16}$/)
      expect(done.transcript.media_path).toContain(done.transcript.download_id!)
    })

    it('the rate-limit "Waiting" event only appears while the setting is on', async () => {
      const { backend } = setup()
      await backend.settingsUpdate(settingsPatch({ rate_limit_sleep: false }))
      const ev = await collect<SttEventDto>((cb) => backend.transcribe(url('https://example.com/ep.mp3'), cb))
      expect(types(ev)).not.toContain('download_waiting')
      expect(types(ev).at(-1)).toBe('done')
    })

    it('every error class ends the stream with exactly one error event and no download', async () => {
      const cases: Array<[string, string]> = [
        ['https://youtube.com/live', 'download_live'],
        ['https://youtube.com/private', 'download_login'],
        ['https://youtube.com/age', 'download_login'],
        ['https://youtube.com/cookies', 'download_cookies'],
        ['https://open.spotify.com/episode/abc', 'download_drm'],
        ['https://example.com/notfound', 'download_not_found'],
        ['https://example.com/offline', 'download_network'],
        ['https://example.com/noaudio', 'download_no_audio'],
        ['https://example.com/diskfull', 'disk_space'],
        ['https://example.com/fail', 'download_failed'],
      ]
      for (const [u, code] of cases) {
        const { backend } = setup()
        const ev = await collect<SttEventDto>((cb) => backend.transcribe(url(u), cb))
        expect(err(ev).code, u).toBe(code)
        expect(types(ev).filter((t) => t === 'done' || t === 'error'), u).toEqual(['error'])
        expect(types(ev), u).not.toContain('downloaded')
        expect(types(ev), u).not.toContain('paragraph')
        expect((await backend.workspaceGet()).pending_url, `${u}: the link stays for a one-click retry`).toBe(new URL(u).toString())
      }
    })

    it('login errors point at the cookies setting; the generic failure names the install command; a fallback is announced first', async () => {
      const { backend } = setup()
      expect(err(await collect<SttEventDto>((cb) => backend.transcribe(url('https://youtube.com/private'), cb))).message).toContain('Use browser cookies')
      const ev = await collect<SttEventDto>((cb) => backend.transcribe(url('https://example.com/fail'), cb))
      expect(err(ev).message).toContain('brew install yt-dlp')
      expect(ev.find((e) => e.type === 'downloader_fallback')).toMatchObject({ from: 'bundled', to: 'system', message: 'Bundled downloader failed, retried with system yt-dlp' })
    })

    it('a playlist link is flagged (only its first item is used)', async () => {
      const { backend } = setup()
      const ev = await collect<SttEventDto>((cb) => backend.transcribe(url('https://youtube.com/playlist'), cb))
      expect(ev.find((e) => e.type === 'download_info')).toMatchObject({ playlist: true })
    })

    it('the link is remembered while downloading, forgotten once the audio is on disk, and cleared by an explicit cancel', async () => {
      const { backend } = setup({ timeScale: 0.05 })
      const ev: SttEventDto[] = []
      const id = await backend.transcribe(url('https://example.com/ep.mp3'), (e) => ev.push(e))
      expect((await backend.workspaceGet()).pending_url).toBe('https://example.com/ep.mp3')
      await backend.jobCancel(id)
      await new Promise((r) => setTimeout(r, 600))
      expect(err(ev).code).toBe('cancelled')
      expect(types(ev)).not.toContain('downloaded')
      expect((await backend.workspaceGet()).pending_url).toBeNull()

      const { backend: b2 } = setup()
      await collect<SttEventDto>((cb) => b2.transcribe(url('https://example.com/ep.mp3'), cb))
      expect((await b2.workspaceGet()).pending_url).toBeNull()
    })

    it('invalid links and the off switch refuse before any job starts', async () => {
      const { backend } = setup()
      for (const bad of ['', 'not a link', 'ftp://example.com/a.mp3', 'file:///etc/passwd']) {
        await expect(backend.transcribe(url(bad), () => {}), bad).rejects.toMatchObject({ code: 'invalid_url' })
      }
      await backend.settingsUpdate(settingsPatch({ allow_downloads: false }))
      await expect(backend.transcribe(url('https://example.com/ep.mp3'), () => {})).rejects.toMatchObject({ code: 'download_disabled' })
      await expect(backend.downloadsInfo()).rejects.toMatchObject({ code: 'download_disabled' })
      // local files still work with downloads off
      expect(types(await collect<SttEventDto>((cb) => backend.transcribe({ source: { kind: 'path', path: '/a/x.mp3' }, language: 'auto' }, cb))).at(-1)).toBe('done')
    })

    it('re-transcribing the downloaded audio keeps its link; export and info work', async () => {
      const { backend } = setup()
      const first = (await collect<SttEventDto>((cb) => backend.transcribe(url('https://example.com/ep.mp3'), cb))).at(-1) as Extract<SttEventDto, { type: 'done' }>
      const t = first.transcript
      await backend.workspaceUpdate({ transcript: { text: t.text, source_name: t.source_name, language: 'en', duration_s: t.duration_s, words: t.words, edited: false, exported_or_copied: false, recording_id: null, source_url: t.source_url, download_id: t.download_id, media_path: t.media_path }, tts: null, clear_transcript: false, clear_tts: false })
      const again = (await collect<SttEventDto>((cb) => backend.transcribe({ source: { kind: 'path', path: t.media_path! }, language: 'de' }, cb))).at(-1) as Extract<SttEventDto, { type: 'done' }>
      expect(again.transcript.download_id).toBe(t.download_id)
      expect(again.transcript.source_url).toBe(t.source_url)
      expect(await backend.exportDownload(t.download_id!, null, '/Users/me/ep.m4a')).toBe('/Users/me/ep.m4a')
      expect(await backend.exportDownload(t.download_id!, 'mp3', '/Users/me/ep')).toBe('/Users/me/ep.mp3')
      const info = await backend.downloadsInfo()
      expect(info.tools[0]).toMatchObject({ kind: 'bundled', version: '2026.08.19', impersonate: true })
      expect(info.browsers.map((b) => b.id)).toContain('safari')
    })

    it('download settings merge, and "" clears the path and the browser', async () => {
      const { backend } = setup()
      let s = await backend.settingsGet()
      expect(s).toMatchObject({ allow_downloads: true, impersonate: true, rate_limit_sleep: true, use_browser_cookies: false, cookies_browser: null, downloader_path: null })
      s = await backend.settingsUpdate(settingsPatch({ use_browser_cookies: true, cookies_browser: 'firefox', downloader_path: '/opt/homebrew/bin/yt-dlp', impersonate: false }))
      expect(s).toMatchObject({ use_browser_cookies: true, cookies_browser: 'firefox', downloader_path: '/opt/homebrew/bin/yt-dlp', impersonate: false })
      s = await backend.settingsUpdate(settingsPatch({ cookies_browser: '', downloader_path: '' }))
      expect(s.cookies_browser).toBeNull()
      expect(s.downloader_path).toBeNull()
    })
  })
})
