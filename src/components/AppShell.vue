<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { serverInfo, type ServerInfo } from '../lib/server'
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
      <div class="section-label side-label">EB MCP</div>
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
      <header class="toolbar" data-tauri-drag-region>
        <h1>{{ title }}</h1>
        <span id="toolbar-actions" class="toolbar-actions" />
      </header>
      <div class="content">
        <RouterView />
      </div>
    </main>
  </div>
</template>

<style scoped>
.shell { display: flex; height: 100%; }
.sidebar {
  width: 216px; flex: none; box-sizing: border-box; display: flex; flex-direction: column;
  padding: 0 10px 12px;
}
.traffic-space { height: 52px; flex: none; }
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
.main {
  flex: 1; min-width: 0; display: flex; flex-direction: column; background: var(--content);
  border-left: 0.5px solid var(--line-soft);
}
.toolbar { height: 52px; flex: none; display: flex; align-items: center; padding: 0 20px; }
.toolbar h1 { font-size: 15px; font-weight: 700; }
.toolbar-actions { margin-left: auto; display: flex; align-items: center; gap: 8px; }
.content { flex: 1; min-height: 0; overflow: auto; }
</style>
