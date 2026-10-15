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

  it('voices: the ten English voices (George among them), no phantom ctric voices', async () => {
    const { backend } = setup()
    const v = await backend.voices()
    expect(v).toHaveLength(10)
    expect(v.map((x) => x.id)).toContain('george')
    expect(v.map((x) => x.id)).not.toContain('bill_boerst')
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
    await backend.workspaceUpdate({ transcript: { text: 'hi', source_name: 'a.m4a', language: 'en', duration_s: 3, words: 1, edited: false, exported_or_copied: false, recording_id: null }, tts: null, clear_transcript: false, clear_tts: false })
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
})
