import { createPinia } from 'pinia'
import { createApp, nextTick } from 'vue'
import App from './App.vue'
import { backend, initBackend } from './backend'
import { hydrate, registerInteractions } from './lib/bootstrap'
import './styles/base.css'

async function start() {
  await initBackend()
  const app = createApp(App).use(createPinia())
  // Pinia is active once installed; stores can be used from here on.
  await registerInteractions()
  // Load the persisted state before the first paint: mounting first showed the empty screen for a frame or two and then
  // swapped in the restored transcript (a visible flash when the window is reopened). A failed load must not leave a
  // blank window, so mount regardless.
  try {
    await hydrate()
  } finally {
    app.mount('#app')
    // the window is created hidden; show it only now, with the restored screen already in the DOM
    await nextTick()
    await backend().uiReady()
  }
}

void start()
