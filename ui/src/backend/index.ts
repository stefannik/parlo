import type { Backend } from './types'

let current: Backend | null = null

/** The one backend instance. Tests (and the mock build) install theirs with `setBackend` before the app mounts. */
export function setBackend(b: Backend) {
  current = b
}

export function backend(): Backend {
  if (!current) throw new Error('backend not initialised: call setBackend() (or initBackend()) first')
  return current
}

/** Picks the mock (VITE_MOCK_BACKEND=1) or the real Tauri adapter. Dynamic imports keep the unused one out of the bundle. */
export async function initBackend(): Promise<Backend> {
  if (import.meta.env.VITE_MOCK_BACKEND === '1') {
    const { MockBackend } = await import('./mock')
    const params = new URLSearchParams(window.location.search)
    const mock = new MockBackend({
      timeScale: Number(params.get('timescale') ?? '1'),
      micPermission: (params.get('mic') as never) ?? 'granted',
      // ?recover=1 starts with an interrupted recording and TTS job listed (screenshots / manual testing)
      recoverables:
        params.get('recover') === '1'
          ? [
              { kind: 'recording', id: 'rec-recovered', wav_path: '/mock/recordings/rec-recovered.wav', duration_ms: 1_845_000, started_at: new Date(Date.now() - 3_600_000).toISOString() },
              { kind: 'tts_job', job_id: 'job-1', text_preview: 'Chapter 3 — The long way round', voice: 'george' },
            ]
          : undefined,
    })
    ;(window as unknown as { __parloMock: unknown }).__parloMock = mock
    setBackend(mock)
    return mock
  }
  const { RealBackend } = await import('./real')
  const real = new RealBackend()
  setBackend(real)
  return real
}

export { BackendError } from './types'
export type { Backend } from './types'
