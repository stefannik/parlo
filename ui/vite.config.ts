import { defineConfig } from 'vitest/config'
import vue from '@vitejs/plugin-vue'

// Tauri expects a fixed dev port; the mock backend (VITE_MOCK_BACKEND=1) lets the UI run in a plain browser.
export default defineConfig({
  plugins: [vue()],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: { target: 'safari16', sourcemap: false },
  test: { environment: 'jsdom', globals: true, setupFiles: ['src/test/setup.ts'] },
})
