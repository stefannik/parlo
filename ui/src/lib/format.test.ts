import { describe, expect, it } from 'vitest'
import { classifyPath, countWords, formatBytes, formatClock, formatDurationWords, formatEta, formatListening, listeningMinutes, stripExt } from './format'

describe('format', () => {
  it('formats clocks', () => {
    expect(formatClock(0)).toBe('0:00')
    expect(formatClock(65_000)).toBe('1:05')
    expect(formatClock(3_725_000)).toBe('1:02:05')
  })
  it('formats durations in words', () => {
    expect(formatDurationWords(42)).toBe('42 s')
    expect(formatDurationWords(120)).toBe('2 min')
    expect(formatDurationWords(252)).toBe('4 min 12 s')
    expect(formatDurationWords(3900)).toBe('1 h 05 min')
  })
  it('formats ETA and bytes', () => {
    expect(formatEta(null)).toBe('')
    expect(formatEta(30_000)).toBe('30 s left')
    expect(formatEta(95_000)).toBe('1 min 35 s left')
    expect(formatBytes(512)).toBe('512 B')
    expect(formatBytes(874_000_000)).toBe('834 MB')
  })
  it('counts words and estimates listening time', () => {
    expect(countWords('  hello   world\nfoo ')).toBe(3)
    expect(countWords('')).toBe(0)
    expect(listeningMinutes(150)).toBe(1)
    expect(listeningMinutes(150, 2)).toBe(0.5)
    expect(formatListening(0.05)).toBe('< 10 s')
    expect(formatListening(2.4)).toBe('2 min')
    expect(formatListening(90)).toBe('1 h 30 min')
  })
  it('classifies dropped files', () => {
    expect(classifyPath('/a/b/Talk.M4A')).toBe('media')
    expect(classifyPath('C:\\x\\clip.mkv')).toBe('media')
    expect(classifyPath('/a/book.epub')).toBe('document')
    expect(classifyPath('/a/archive.zip')).toBe('unknown')
    expect(stripExt('a.b.txt')).toBe('a.b')
  })
})
