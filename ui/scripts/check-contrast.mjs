// WCAG contrast check of the design tokens (light + dark). Fails (exit 1) if any listed pair is below its threshold.
//   node scripts/check-contrast.mjs
import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const css = readFileSync(join(dirname(fileURLToPath(import.meta.url)), '../src/styles/tokens.css'), 'utf8')

function block(re) {
  const m = css.match(re)
  if (!m) throw new Error('block not found: ' + re)
  const out = {}
  for (const [, k, v] of m[1].matchAll(/--([\w-]+):\s*([^;]+);/g)) out[k] = v.trim()
  return out
}
const light = block(/:root\s*\{([\s\S]*?)\n\}/)
const dark = { ...light, ...block(/:root\[data-theme="dark"\]\s*\{([\s\S]*?)\n\}/) }

function parse(c) {
  c = c.trim()
  let m = c.match(/^#([0-9a-f]{6})$/i)
  if (m) return [0, 2, 4].map((i) => parseInt(m[1].slice(i, i + 2), 16)).concat(1)
  m = c.match(/^rgba?\(([^)]+)\)$/)
  if (m) {
    const p = m[1].split(',').map((x) => parseFloat(x))
    return [p[0], p[1], p[2], p[3] ?? 1]
  }
  throw new Error('cannot parse ' + c)
}
const over = (fg, bg) => fg.slice(0, 3).map((v, i) => v * fg[3] + bg[i] * (1 - fg[3]))
const lum = ([r, g, b]) => {
  const f = (v) => ((v /= 255) <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4)
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b)
}
const ratio = (a, b) => {
  const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p)
  return (x + 0.05) / (y + 0.05)
}

// [foreground token, background token, minimum ratio, label]. Translucent backgrounds are composited over --surface.
const PAIRS = [
  ['text', 'bg', 7, 'body text on window'],
  ['text', 'surface', 7, 'body text on cards'],
  ['text-2', 'surface', 4.5, 'secondary text on cards'],
  ['text-2', 'bg', 4.5, 'secondary text on window'],
  ['text-3', 'surface', 3, 'tertiary text / placeholders on cards (large or decorative)'],
  ['on-accent', 'accent', 4.5, 'text on primary buttons'],
  ['accent-text', 'surface', 4.5, 'accent-coloured text on cards'],
  ['accent-text', 'accent-soft', 4.5, 'accent-coloured text on accent chip (over surface)'],
  ['accent', 'surface', 3, 'accent as a graphic (focus ring, progress fill, icons)'],
  ['danger', 'surface', 4.5, 'danger text on cards'],
  ['danger', 'danger-soft', 4.5, 'danger text on danger chip'],
  ['success', 'surface', 4.5, 'success text on cards'],
  ['text', 'warning-soft', 4.5, 'text on warning banner'],
  ['text', 'accent-soft', 4.5, 'text on info banner'],
  ['text-2', 'surface-2', 4.5, 'secondary text on chips / segmented control'],
  ['text', 'surface-2', 7, 'selected segment / chips'],
]
let bad = 0
for (const [name, theme] of [['light', light], ['dark', dark]]) {
  for (const [f, b, min, label] of PAIRS) {
    const surface = parse(theme.surface)
    const bg = over(parse(theme[b]), surface.concat(1).slice(0, 3)) // composite translucent over surface
    const fg = over(parse(theme[f]), bg)
    const r = ratio(fg, bg)
    const ok = r >= min
    if (!ok) bad++
    console.log(`${ok ? 'ok  ' : 'FAIL'} ${name.padEnd(5)} ${r.toFixed(2).padStart(5)} (min ${min})  ${label}  [--${f} on --${b}]`)
  }
}
process.exit(bad ? 1 : 0)
