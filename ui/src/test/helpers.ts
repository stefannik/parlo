import { createPinia, setActivePinia } from 'pinia'
import { MockBackend, type MockOptions } from '../backend/mock'
import { setBackend } from '../backend'

export function setup(opts: MockOptions = {}) {
  const pinia = createPinia()
  setActivePinia(pinia)
  const backend = new MockBackend({ timeScale: 0, ...opts })
  setBackend(backend)
  return { pinia, backend }
}

export const flush = () => new Promise<void>((r) => setTimeout(r, 0))
