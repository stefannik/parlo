export function formatClock(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000))
  const h = Math.floor(total / 3600)
  const m = Math.floor((total % 3600) / 60)
  const s = total % 60
  const mm = h > 0 ? String(m).padStart(2, '0') : String(m)
  return h > 0 ? `${h}:${mm}:${String(s).padStart(2, '0')}` : `${mm}:${String(s).padStart(2, '0')}`
}

/** "3 min 20 s", "45 s", "1 h 05 min" — for durations spoken in sentences. */
export function formatDurationWords(seconds: number): string {
  const s = Math.max(0, Math.round(seconds))
  if (s < 60) return `${s} s`
  const m = Math.floor(s / 60)
  if (m < 60) return s % 60 === 0 ? `${m} min` : `${m} min ${s % 60} s`
  const h = Math.floor(m / 60)
  return `${h} h ${String(m % 60).padStart(2, '0')} min`
}

export function formatEta(ms: number | null | undefined): string {
  if (ms == null) return ''
  const s = Math.max(1, Math.round(ms / 1000))
  if (s < 60) return `${s} s left`
  const m = Math.floor(s / 60)
  if (m < 60) return `${m} min ${String(s % 60).padStart(2, '0')} s left`
  return `${Math.floor(m / 60)} h ${String(m % 60).padStart(2, '0')} min left`
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`
  const units = ['KB', 'MB', 'GB', 'TB']
  let v = n / 1024
  let i = 0
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024
    i++
  }
  return `${v >= 100 ? v.toFixed(0) : v.toFixed(1)} ${units[i]}`
}

export function countWords(text: string): number {
  const m = text.trim().match(/\S+/g)
  return m ? m.length : 0
}

/** Listening time at 1× (150 wpm, PocketTTS's natural rate), optionally scaled by playback speed. */
export function listeningMinutes(words: number, speed = 1): number {
  return words / 150 / speed
}

/** "under 10 s", "~45 s", "~3 min", "~1 h 30 min" — an estimate, so always approximate. */
export function formatListening(minutes: number): string {
  if (minutes < 1 / 6) return 'under 10 s'
  if (minutes < 1) return `~${Math.round(minutes * 60)} s`
  if (minutes < 60) return `~${Math.round(minutes)} min`
  return `~${Math.floor(minutes / 60)} h ${String(Math.round(minutes % 60)).padStart(2, '0')} min`
}

export function baseName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path
}

export function stripExt(name: string): string {
  const i = name.lastIndexOf('.')
  return i > 0 ? name.slice(0, i) : name
}

const AUDIO_VIDEO = new Set(['wav', 'mp3', 'm4a', 'aac', 'flac', 'ogg', 'oga', 'opus', 'aiff', 'aif', 'caf', 'mp4', 'mov', 'm4v', 'mkv', 'webm', 'avi', 'wma', 'amr', '3gp'])
const DOCUMENTS = new Set(['txt', 'md', 'markdown', 'docx', 'pdf', 'epub', 'text'])

export type DroppedKind = 'media' | 'document' | 'unknown'
export function classifyPath(path: string): DroppedKind {
  const ext = baseName(path).split('.').pop()?.toLowerCase() ?? ''
  if (AUDIO_VIDEO.has(ext)) return 'media'
  if (DOCUMENTS.has(ext)) return 'document'
  return 'unknown'
}
