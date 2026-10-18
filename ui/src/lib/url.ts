// Link handling for "Transcribe from URL". Mirrors the rules of `parlo_core::download::url::validate_url` (the core
// re-validates on start); used for instant feedback, paste/drop detection and fallback titles.

export const MAX_URL_LENGTH = 2048

/** The normalized URL, or `null` when the text isn't a single http(s) link. */
export function parseHttpUrl(text: string): string | null {
  return checkUrl(text).url
}

/** A human message about what's wrong with `text`, or `null` when it is a usable link (or empty). */
export function urlProblem(text: string): string | null {
  if (!text.trim()) return null
  return checkUrl(text).problem
}

export function checkUrl(text: string): { url: string | null; problem: string | null } {
  const s = text.trim()
  const bad = (problem: string) => ({ url: null, problem })
  if (!s) return bad('Paste a link first.')
  if (s.length > MAX_URL_LENGTH) return bad('That link is too long.')
  if (/[\s\u0000-\u001f]/.test(s)) return bad('That doesn’t look like a single web link (it contains spaces).')
  let u: URL
  try {
    u = new URL(s)
  } catch {
    return bad('That doesn’t look like a web link. It should start with http:// or https://')
  }
  if (u.protocol !== 'http:' && u.protocol !== 'https:') return bad(`Only http:// and https:// links are supported (this one is ${u.protocol}//).`)
  if (!u.hostname) return bad('That link has no website name.')
  return { url: u.toString(), problem: null }
}

/** `www.` stripped host of a link (for display). */
export function hostOf(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, '')
  } catch {
    return url
  }
}

/** Title to show before the site tells us the real one: `file name (host)`, or the host. */
export function fallbackTitle(url: string): string {
  const host = hostOf(url)
  try {
    const seg = new URL(url).pathname.split('/').filter(Boolean).pop()
    if (seg) return `${decodeURIComponent(seg)} (${host})`
  } catch {
    /* fall through */
  }
  return host
}

/** First link in a dragged `text/uri-list` or plain-text payload. */
export function linkFromDrop(uriList: string, plain: string): string | null {
  for (const line of uriList.split(/\r?\n/)) {
    if (line && !line.startsWith('#')) {
      const u = parseHttpUrl(line)
      if (u) return u
    }
  }
  return parseHttpUrl(plain)
}
