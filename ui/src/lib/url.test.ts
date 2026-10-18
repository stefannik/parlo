import { describe, expect, it } from 'vitest'
import { checkUrl, fallbackTitle, hostOf, linkFromDrop, parseHttpUrl, urlProblem } from './url'

describe('link validation', () => {
  it('accepts http and https links and trims them', () => {
    expect(parseHttpUrl('  https://www.youtube.com/watch?v=abc  ')).toBe('https://www.youtube.com/watch?v=abc')
    expect(parseHttpUrl('http://127.0.0.1:8080/a.mp3')).toBe('http://127.0.0.1:8080/a.mp3')
    expect(parseHttpUrl('https://example.com')).toBe('https://example.com/')
  })

  it('rejects everything else with a useful message', () => {
    const cases: Array<[string, RegExp]> = [
      ['youtube.com/watch?v=abc', /start with http/],
      ['ftp://example.com/a.mp3', /Only http/],
      ['file:///etc/passwd', /Only http/],
      ['javascript:alert(1)', /Only http/],
      ['https://exa mple.com', /single web link/],
      ['two https://a.com links', /single web link/],
      ['hello', /start with http/],
    ]
    for (const [text, msg] of cases) {
      expect(parseHttpUrl(text), text).toBeNull()
      expect(urlProblem(text), text).toMatch(msg)
    }
    expect(urlProblem('')).toBeNull() // empty is not an error, just nothing yet
    expect(checkUrl('').problem).toBe('Paste a link first.')
    expect(parseHttpUrl(`https://a.com/${'x'.repeat(3000)}`)).toBeNull()
  })

  it('titles fall back to file and host', () => {
    expect(fallbackTitle('https://cdn.example.com/pod/Episode%2012.mp3?x=1')).toBe('Episode 12.mp3 (cdn.example.com)')
    expect(fallbackTitle('https://www.example.com/')).toBe('example.com')
    expect(hostOf('https://www.youtube.com/watch?v=1')).toBe('youtube.com')
  })

  it('finds a dropped link in uri-list or plain text', () => {
    expect(linkFromDrop('# comment\r\nhttps://a.com/x\r\n', '')).toBe('https://a.com/x')
    expect(linkFromDrop('', 'https://b.com/y')).toBe('https://b.com/y')
    expect(linkFromDrop('', 'just some text')).toBeNull()
    expect(linkFromDrop('file:///Users/me/a.mp3', '')).toBeNull()
  })
})
