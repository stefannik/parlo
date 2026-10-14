import { describe, expect, it, vi } from 'vitest'
import { setup } from '../test/helpers'
import { useSpeakStore } from './speak'

const LONG = 'This is a sentence that is spoken aloud. '.repeat(80)

async function settled(sp: ReturnType<typeof useSpeakStore>) {
  await vi.waitFor(() => expect(['result', 'error', 'compose']).toContain(sp.phase), { timeout: 3000 })
}

describe('speak store state machine', () => {
  it('compose → generating → result; segments arrive before done', async () => {
    setup()
    const sp = useSpeakStore()
    await sp.loadVoices()
    expect(sp.canGenerate).toBe(false)
    sp.setText(LONG)
    expect(sp.canGenerate).toBe(true)
    const p = sp.generate()
    expect(sp.phase).toBe('generating')
    await p
    await settled(sp)
    expect(sp.phase).toBe('result')
    expect(sp.segments.length).toBeGreaterThan(0)
    expect(sp.outputId).toMatch(/^out-/)
    expect(sp.durationMs).toBeGreaterThan(0)
  })

  it('shows segments while still generating (listen while it generates)', async () => {
    const { backend } = setup({ timeScale: 0.05 })
    void backend
    const sp = useSpeakStore()
    await sp.loadVoices()
    sp.setText(LONG)
    await sp.generate()
    await vi.waitFor(() => expect(sp.segments.length).toBeGreaterThan(0), { timeout: 3000 })
    expect(sp.phase).toBe('generating')
    await settled(sp)
  })

  it('marks the output out of date when the text or the voice changes, and clears on regenerate', async () => {
    setup()
    const sp = useSpeakStore()
    await sp.loadVoices()
    sp.setText(LONG)
    await sp.generate()
    await settled(sp)
    expect(sp.outOfDate).toBe(false)
    sp.setText(LONG + ' One more.')
    expect(sp.outOfDate).toBe(true)
    sp.setText(LONG)
    expect(sp.outOfDate).toBe(false)
    sp.voice = 'alba'
    expect(sp.outOfDate).toBe(true)
    await sp.generate()
    await settled(sp)
    expect(sp.outOfDate).toBe(false)
  })

  it('speed is playback-only: changing it never makes the output stale', async () => {
    setup()
    const sp = useSpeakStore()
    await sp.loadVoices()
    sp.setText(LONG)
    await sp.generate()
    await settled(sp)
    sp.speed = 1.75
    expect(sp.outOfDate).toBe(false)
  })

  it('cancel while generating returns to compose with nothing playable', async () => {
    setup({ timeScale: 0.1 })
    const sp = useSpeakStore()
    await sp.loadVoices()
    sp.setText(LONG)
    await sp.generate()
    await sp.cancel()
    await settled(sp)
    expect(sp.phase).toBe('compose')
    expect(sp.hasOutput).toBe(false)
  })

  it('error shows the message and Retry works', async () => {
    setup({ failNextTts: true })
    const sp = useSpeakStore()
    await sp.loadVoices()
    sp.setText('Hello world.')
    await sp.generate()
    await settled(sp)
    expect(sp.phase).toBe('error')
    expect(sp.error?.code).toBe('model_missing')
    await sp.generate()
    await settled(sp)
    expect(sp.phase).toBe('result')
  })

  it('import fills the editor and shows a source chip; editing detaches it', async () => {
    setup()
    const sp = useSpeakStore()
    await sp.importFile('/docs/report.pdf')
    expect(sp.imported?.file_name).toBe('report.pdf')
    expect(sp.text.length).toBeGreaterThan(100)
    expect(sp.words).toBeGreaterThan(20)
    sp.setText(sp.text + ' edited')
    expect(sp.imported).toBeNull()
  })

  it('large documents are read-only previews and use the import id as the TTS source', async () => {
    const { backend } = setup()
    const sp = useSpeakStore()
    await sp.loadVoices()
    vi.spyOn(backend, 'importText').mockResolvedValueOnce({
      import_id: 'imp-big', file_name: 'book.epub', kind: 'epub', text: null, preview: 'Chapter 1 …', large: true,
      chars: 900_000, words: 150_000, pages: null, chapters: 30, est_minutes: 1000, warnings: [],
    })
    const spy = vi.spyOn(backend, 'tts')
    await sp.importFile('/docs/book.epub')
    expect(sp.readOnly).toBe(true)
    expect(sp.words).toBe(150_000)
    expect(sp.canGenerate).toBe(true)
    await sp.generate()
    expect(spy.mock.calls[0]![0].source).toEqual({ kind: 'import', import_id: 'imp-big' })
    await sp.cancel()
  })

  it('counts words/characters/listening time from the editor text', async () => {
    setup()
    const sp = useSpeakStore()
    sp.setText('one two three four five six')
    expect(sp.words).toBe(6)
    expect(sp.chars).toBe(27)
    expect(sp.minutes1x).toBeCloseTo(6 / 150)
    sp.speed = 2
    expect(sp.minutesAtSpeed).toBeCloseTo(3 / 150)
  })

  it('restores the last output and text from the workspace', async () => {
    setup()
    const sp = useSpeakStore()
    await sp.loadVoices()
    sp.restore({ text: 'Saved text.', voice: 'mary', output_id: 'out-1', source_name: null, segments: ['/o/a.wav', '/o/b.wav'], duration_ms: 20000 })
    expect(sp.phase).toBe('result')
    expect(sp.segments).toHaveLength(2)
    expect(sp.voice).toBe('mary')
    expect(sp.outOfDate).toBe(false)
  })
})
