<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import Alert from './Alert.vue'
import { inApp, quitApp, serverInfo, type ServerInfo } from '../lib/server'
import Icon from './Icon.vue'
import UpdateUi from './UpdateUi.vue'

const nav = [
  { to: '/dashboard', icon: 'chart', label: 'Dashboard' },
  { to: '/accounts', icon: 'users', label: 'Accounts' },
  { to: '/verzoeken', icon: 'list', label: 'Verzoeken' },
  { to: '/koppelen', icon: 'plug', label: 'Koppelen' },
]

const route = useRoute()
const router = useRouter()

// Cmd+, (Ctrl+, on Windows) opens Instellingen, as in every Mac app.
function onKey(event: KeyboardEvent) {
  if (event.key === ',' && (event.metaKey || event.ctrlKey)) {
    event.preventDefault()
    void router.push('/instellingen')
  }
}
onMounted(() => window.addEventListener('keydown', onKey))

// Stop EB MCP (Cmd+Q) from the menu bar while AI calls still run: the app asks here first (lib.rs, QUIT_REQUESTED).
const quitCalls = ref(0)
const quitting = ref(false)
onMounted(async () => {
  if (!inApp()) return
  const { listen } = await import('@tauri-apps/api/event')
  await listen<number>('quit-requested', (event) => (quitCalls.value = event.payload))
})
async function confirmQuit() {
  quitting.value = true
  await quitApp()
}
onBeforeUnmount(() => window.removeEventListener('keydown', onKey))
const title = computed(() => String(route.meta.title ?? ''))
const info = ref<ServerInfo | null>(null)
onMounted(async () => {
  info.value = await serverInfo()
})
</script>

<template>
  <div class="shell">
    <nav class="sidebar" aria-label="Hoofdmenu" data-tauri-drag-region>
      <div class="traffic-space" data-tauri-drag-region />
      <div class="section-label side-label" data-tauri-drag-region>EB MCP</div>
      <RouterLink v-for="item in nav" :key="item.to" :to="item.to" class="nav" active-class="active">
        <Icon :name="item.icon" class="nav-icon" />
        <span>{{ item.label }}</span>
      </RouterLink>
      <div class="side-foot">
        <UpdateUi />
        <RouterLink to="/instellingen" class="nav settings" active-class="active">
          <Icon name="settings" class="nav-icon" />
          <span>Instellingen</span>
          <span class="shortcut">⌘,</span>
        </RouterLink>
        <span class="server-line">
          <span class="dot" :class="info?.running ? 'on' : 'bad'" />
          {{ info?.running ? 'MCP actief' : 'MCP staat uit' }}
        </span>
        <span class="mono server-addr">{{ info?.error ?? '127.0.0.1:47821' }}</span>
      </div>
    </nav>
    <main class="main">
      <!-- deep: the title drags too; buttons in the toolbar stay clickable. -->
      <header class="toolbar" data-tauri-drag-region="deep">
        <h1>{{ title }}</h1>
        <span id="toolbar-actions" class="toolbar-actions" />
      </header>
      <div class="content">
        <RouterView />
      </div>
    </main>
  </div>
  <Transition name="modal">
    <Alert v-if="quitCalls" title="Toch afsluiten?" tone="warn" @close="quitCalls = 0">
      <p>{{ quitCalls === 1 ? 'Er loopt nog 1 aanroep' : `Er lopen nog ${quitCalls} aanroepen` }} van een AI-client. Die krijgt dan een fout, en alle sessies worden uitgelogd.</p>
      <template #buttons>
        <button type="button" class="btn" @click="quitCalls = 0">Annuleren</button>
        <button type="button" class="btn danger-fill" :disabled="quitting" :aria-busy="quitting" @click="confirmQuit"><span class="btn-label">Afsluiten</span></button>
      </template>
    </Alert>
  </Transition>
</template>

<style scoped>
.shell { display: flex; height: 100%; background: var(--glass-window); }
/* A floating glass sidebar, inset from the window edge, with the traffic lights inside it (the boards' window()). */
.sidebar {
  width: 216px; flex: none; box-sizing: border-box; display: flex; flex-direction: column;
  margin: 8px 0 8px 8px; padding: 0 10px 10px; border-radius: 14px;
  background: var(--glass-side); box-shadow: var(--glass-side-edge);
  -webkit-backdrop-filter: blur(30px) saturate(180%); backdrop-filter: blur(30px) saturate(180%);
}
.traffic-space { height: 44px; flex: none; }
.side-label { padding: 6px 8px 4px; }
.nav {
  box-sizing: border-box; height: 28px; display: flex; align-items: center; gap: 8px; padding: 0 8px;
  border-radius: 8px; color: var(--text); text-decoration: none; transition: background-color 150ms ease;
}
.nav:hover { background: var(--hover); }
.nav.active { background: var(--selected); }
.nav-icon { color: var(--accent); }
.settings { margin: 0 -4px 4px; }
.shortcut { margin-left: auto; font-size: 11px; color: var(--muted); }
.side-foot { margin-top: auto; display: flex; flex-direction: column; gap: 6px; padding: 0 4px; }
.server-line { display: flex; align-items: center; gap: 7px; font-size: 12px; font-weight: 500; }
.server-addr { font-size: 11px; color: var(--muted); padding-left: 14px; }
.main { flex: 1; min-width: 0; display: flex; flex-direction: column; }
.toolbar { height: 52px; flex: none; display: flex; align-items: center; padding: 0 20px; }
.toolbar h1 { font-size: 15px; font-weight: 700; }
.toolbar-actions { margin-left: auto; display: flex; align-items: center; gap: 8px; }
.content { flex: 1; min-height: 0; overflow: auto; }
</style>
