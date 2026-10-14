import { createPinia } from 'pinia'
import { createApp } from 'vue'
import App from './App.vue'
import { initBackend } from './backend'
import { hydrate, registerInteractions } from './lib/bootstrap'
import './styles/base.css'

async function start() {
  await initBackend()
  const app = createApp(App).use(createPinia())
  // Pinia is active once installed; stores can be used from here on.
  app.mount('#app')
  await registerInteractions()
  await hydrate()
}

void start()
