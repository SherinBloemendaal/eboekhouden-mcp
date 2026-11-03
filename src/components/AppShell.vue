<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRoute } from 'vue-router'
import { serverInfo, type ServerInfo } from '../lib/server'
import Icon from './Icon.vue'

const nav = [
  { to: '/dashboard', icon: 'chart', label: 'Dashboard' },
  { to: '/accounts', icon: 'users', label: 'Accounts' },
  { to: '/verzoeken', icon: 'list', label: 'Verzoeken' },
  { to: '/koppelen', icon: 'plug', label: 'Koppelen' },
]

const route = useRoute()
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
.side-foot { margin-top: auto; display: flex; flex-direction: column; gap: 3px; padding: 0 4px; }
.server-line { display: flex; align-items: center; gap: 7px; font-size: 12px; font-weight: 500; }
.server-addr { font-size: 11px; color: var(--muted); padding-left: 14px; }
.main {
  flex: 1; min-width: 0; display: flex; flex-direction: column; background: var(--content);
  border-left: 0.5px solid var(--line-soft);
}
.toolbar { height: 52px; flex: none; display: flex; align-items: center; padding: 0 20px; }
.toolbar h1 { font-size: 15px; font-weight: 700; }
.content { flex: 1; min-height: 0; overflow: auto; }
</style>
