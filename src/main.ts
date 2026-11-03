import { createApp } from 'vue'
import App from './App.vue'
import { router } from './router'
import './styles/tokens.css'
import './styles/base.css'
import './styles/windows.css'

// One code base, two skins: macOS glass by default, Fluent on Windows (styles/windows.css).
if (navigator.userAgent.includes('Windows')) document.documentElement.dataset.platform = 'windows'

createApp(App).use(router).mount('#app')
