// Screenshot tool (BRIEF §11.6): runs the UI in mock mode and captures every state in light and dark with Playwright's
// WebKit engine (the closest thing to WKWebView available on Linux).
//   node shoot.mjs [outDir]        (default: ./out)
import { spawn } from 'node:child_process'
import { mkdirSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { webkit } from 'playwright'

const here = dirname(fileURLToPath(import.meta.url))
const ui = resolve(here, '../../ui')
const out = resolve(process.argv[2] ?? join(here, 'out'))
const PORT = 5199
const BASE = `http://127.0.0.1:${PORT}/`
mkdirSync(out, { recursive: true })

const server = spawn(process.execPath, [join(ui, 'node_modules/vite/bin/vite.js'), '--port', String(PORT), '--strictPort', '--host', '127.0.0.1'], {
  cwd: ui,
  env: { ...process.env, VITE_MOCK_BACKEND: '1' },
  stdio: ['ignore', 'pipe', 'inherit'],
})
async function waitForServer() {
  for (let i = 0; i < 100; i++) {
    try {
      if ((await fetch(BASE)).ok) return
    } catch {}
    await new Promise((r) => setTimeout(r, 200))
  }
  throw new Error('vite did not start')
}

const browser = await (async () => {
  await waitForServer()
  return webkit.launch()
})()

const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const tid = (id) => `[data-testid="${id}"]`

/** Each shot: name, URL query, and a script that drives the page into the state. */
const SHOTS = [
  { name: 'transcribe-empty', query: '', run: async () => {} },
  { name: 'transcribe-empty-mic-denied', query: '?mic=denied', run: async () => {} },
  { name: 'transcribe-drop-hover', query: '', run: async (p) => { await p.evaluate(() => window.__parloMock.simulateHover(true)); await sleep(300) } },
  {
    name: 'transcribe-recording',
    query: '',
    run: async (p) => {
      await p.click(tid('record-button'))
      await p.waitForSelector(tid('rec-timer'))
      await sleep(2300)
      await p.evaluate(() => window.__parloMock.emitWarning({ kind: 'device_changed', from: 'MacBook Pro Microphone', to: "Stefan's AirPods Pro" }))
      await sleep(400)
    },
  },
  {
    name: 'transcribe-recording-paused',
    query: '',
    run: async (p) => {
      await p.click(tid('record-button'))
      await sleep(1600)
      await p.click(tid('pause-button'))
      await sleep(300)
    },
  },
  {
    name: 'transcribe-processing-loading',
    query: '?timescale=1',
    run: async (p) => {
      await p.evaluate(() => window.__parloMock.control.hold('stt:loading_model'))
      await p.click(tid('choose-file'))
      await p.waitForSelector('[data-stage="loading_model"].current', { timeout: 20000 })
      await sleep(700)
    },
  },
  {
    name: 'transcribe-processing',
    query: '?timescale=1',
    run: async (p) => {
      await p.evaluate(() => window.__parloMock.control.hold('stt:transcribing'))
      await p.click(tid('choose-file'))
      await p.waitForSelector(tid('partial'), { timeout: 20000 })
      await sleep(1500)
    },
  },
  {
    name: 'transcribe-result',
    query: '?timescale=0.2',
    run: async (p) => {
      await p.click(tid('choose-file'))
      await p.waitForSelector(tid('transcript-editor'), { timeout: 30000 })
      await sleep(400)
    },
  },
  {
    name: 'transcribe-result-confirm-new',
    query: '?timescale=0.1',
    run: async (p) => {
      await p.click(tid('choose-file'))
      await p.waitForSelector(tid('transcript-editor'), { timeout: 30000 })
      await p.fill(tid('transcript-editor'), 'I edited this transcript by hand.')
      await p.click(tid('new-button'))
      await p.waitForSelector(tid('confirm-dialog'))
      await sleep(400)
    },
  },
  {
    name: 'transcribe-error',
    query: '?timescale=0.1',
    run: async (p) => {
      await p.evaluate(() => (window.__parloMock.control.options.failNextStt = true))
      await p.click(tid('choose-file'))
      await p.waitForSelector(tid('error-card'), { timeout: 20000 })
      await p.click('text=Show details')
      await sleep(300)
    },
  },
  { name: 'recovery-banner', query: '?recover=1', run: async () => { await sleep(300) } },
  { name: 'speak-empty', query: '', run: async (p) => { await p.click(tid('tab-speak')); await sleep(300) } },
  {
    name: 'speak-imported',
    query: '',
    run: async (p) => {
      await p.click(tid('tab-speak'))
      await p.click(tid('open-file'))
      await p.waitForSelector(tid('source-chip'))
      await sleep(400)
    },
  },
  {
    name: 'speak-voice-picker',
    query: '',
    run: async (p) => {
      await p.click(tid('tab-speak'))
      await p.fill(tid('speak-editor'), 'The quick brown fox jumps over the lazy dog. '.repeat(12))
      await p.click(tid('voice-picker'))
      await sleep(400)
    },
  },
  {
    name: 'speak-generating',
    query: '?timescale=1',
    run: async (p) => {
      await p.evaluate(() => window.__parloMock.control.hold('tts:generating'))
      await p.click(tid('tab-speak'))
      await p.click(tid('open-file'))
      await p.waitForSelector(tid('source-chip'))
      await p.click(tid('generate'))
      await p.waitForSelector(tid('player'), { timeout: 20000 })
      await sleep(1200)
    },
  },
  {
    name: 'speak-result',
    query: '?timescale=0.1',
    run: async (p) => {
      await p.click(tid('tab-speak'))
      await p.click(tid('open-file'))
      await p.waitForSelector(tid('source-chip'))
      await p.click(tid('generate'))
      await p.waitForSelector(tid('reveal'), { timeout: 30000 })
      await sleep(400)
    },
  },
  {
    name: 'speak-result-export-menu',
    query: '?timescale=0.1',
    run: async (p) => {
      await p.click(tid('tab-speak'))
      await p.click(tid('open-file'))
      await p.waitForSelector(tid('source-chip'))
      await p.click(tid('generate'))
      await p.waitForSelector(tid('reveal'), { timeout: 30000 })
      await p.click(tid('export-audio'))
      await sleep(400)
    },
  },
  {
    name: 'speak-result-stale',
    query: '?timescale=0.1',
    run: async (p) => {
      await p.click(tid('tab-speak'))
      await p.click(tid('open-file'))
      await p.waitForSelector(tid('source-chip'))
      await p.click(tid('generate'))
      await p.waitForSelector(tid('reveal'), { timeout: 30000 })
      await p.fill(tid('speak-editor'), 'Replacement text that was typed afterwards.')
      await sleep(400)
    },
  },
  {
    name: 'speak-error',
    query: '?timescale=0.1',
    run: async (p) => {
      await p.evaluate(() => (window.__parloMock.control.options.failNextTts = true))
      await p.click(tid('tab-speak'))
      await p.fill(tid('speak-editor'), 'This will fail.')
      await p.click(tid('generate'))
      await p.waitForSelector(tid('speak-error'), { timeout: 20000 })
      await sleep(300)
    },
  },
  ...['general', 'recording', 'transcription', 'speech', 'storage', 'about'].map((s) => ({
    name: `settings-${s}`,
    query: '',
    run: async (p) => {
      await p.click(tid('open-settings'))
      await p.click(tid(`settings-${s}`))
      await sleep(500)
    },
  })),
  {
    name: 'quit-while-recording',
    query: '',
    run: async (p) => {
      await p.click(tid('record-button'))
      await sleep(800)
      await p.evaluate(() => window.__parloMock.simulateQuit())
      await sleep(400)
    },
  },
]

const only = process.argv.slice(3)
let failed = 0
for (const theme of ['light', 'dark']) {
  const ctx = await browser.newContext({ viewport: { width: 960, height: 700 }, deviceScaleFactor: 2, colorScheme: theme, reducedMotion: 'reduce' })
  for (const shot of SHOTS) {
    if (only.length && !only.includes(shot.name)) continue
    const page = await ctx.newPage()
    page.on('pageerror', (e) => console.error(`[${shot.name}/${theme}] page error:`, e.message))
    try {
      await page.goto(BASE + shot.query, { waitUntil: 'networkidle' })
      await page.waitForSelector(tid('transcribe-view'), { state: 'attached' })
      await sleep(250)
      await shot.run(page)
      await page.screenshot({ path: join(out, `${shot.name}.${theme}.png`) })
      console.log('ok', shot.name, theme)
    } catch (e) {
      failed++
      console.error('FAILED', shot.name, theme, e.message.split('\n')[0])
      await page.screenshot({ path: join(out, `FAILED-${shot.name}.${theme}.png`) }).catch(() => {})
    }
    await page.close()
  }
  await ctx.close()
}
await browser.close()
server.kill()
process.exit(failed ? 1 : 0)
