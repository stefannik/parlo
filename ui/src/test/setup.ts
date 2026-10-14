// jsdom has no media playback: stub the HTMLMediaElement methods the player calls.
Object.defineProperty(HTMLMediaElement.prototype, 'pause', { configurable: true, value: () => {} })
Object.defineProperty(HTMLMediaElement.prototype, 'play', { configurable: true, value: () => Promise.resolve() })
Object.defineProperty(HTMLMediaElement.prototype, 'load', { configurable: true, value: () => {} })
