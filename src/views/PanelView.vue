<script setup lang="ts">
import { onMounted, ref } from 'vue'
import Icon from '../components/Icon.vue'
import { openMain, quitApp, serverInfo, type ServerInfo } from '../lib/server'

const info = ref<ServerInfo | null>(null)
onMounted(async () => {
  info.value = await serverInfo()
})

const items = [
  { icon: 'chart', label: 'Dashboard', route: '/dashboard' },
  { icon: 'users', label: 'Accounts beheren', route: '/accounts' },
  { icon: 'list', label: 'Verzoeken', route: '/verzoeken' },
  { icon: 'plug', label: 'Koppelen met Claude of Cursor', route: '/koppelen' },
]
</script>

<template>
  <div class="panel" role="dialog" aria-label="EB MCP">
    <div class="head">
      <img src="/app-icon.png" alt="" width="30" height="30" />
      <span class="name">
        <strong>EB MCP</strong>
        <span>{{ info?.sessions ? `${info.sessions} sessie${info.sessions === 1 ? '' : 's'}` : 'Niet ingelogd' }}</span>
      </span>
      <span class="state" :class="info?.running ? 'ok' : 'wait'">
        <span class="dot" :class="info?.running ? 'on' : 'off'" />{{ info?.running ? 'MCP actief' : 'MCP uit' }}
      </span>
    </div>
    <div class="section-label label">Accounts</div>
    <p class="hint">Nog geen accounts. AI-clients horen nu dat ze eerst moeten wachten tot je inlogt.</p>
    <div class="sep" />
    <button v-for="item in items" :key="item.route" type="button" class="item" @click="openMain(item.route)">
      <Icon :name="item.icon" :size="15" class="item-icon" />
      <span>{{ item.label }}</span>
    </button>
    <div class="sep" />
    <button type="button" class="item" @click="quitApp">
      <Icon name="power" :size="15" class="item-icon" />
      <span>Afsluiten</span>
      <span class="kbd">⌘Q</span>
    </button>
  </div>
</template>

<style scoped>
.panel { height: 100%; box-sizing: border-box; padding: 0 6px 6px; display: flex; flex-direction: column; animation: show 200ms var(--ease) both; }
@keyframes show { from { opacity: 0; transform: scale(0.96); filter: blur(6px); } to { opacity: 1; transform: none; filter: none; } }
.head { display: flex; align-items: center; gap: 10px; padding: 12px 8px 8px; }
.head img { border-radius: 7px; }
.name { display: flex; flex-direction: column; flex: 1; min-width: 0; }
.name span { font-size: 11px; color: var(--muted); }
.state { display: inline-flex; align-items: center; gap: 5px; height: 20px; padding: 0 8px; border-radius: 999px; font-size: 11px; font-weight: 500; }
.state.ok { background: var(--success-soft); color: var(--success); }
.state.wait { background: var(--subtle); color: var(--label); }
.label { padding: 6px 10px 3px; }
.hint { padding: 2px 10px 6px; font-size: 11px; line-height: 15px; color: var(--muted); }
.sep { height: 1px; background: var(--line-soft); margin: 5px 10px; }
.item {
  box-sizing: border-box; width: 100%; min-height: 26px; display: flex; align-items: center; gap: 8px; padding: 0 8px;
  border: 0; border-radius: 8px; background: transparent; text-align: left; transition: background-color 120ms ease;
}
.item:hover { background: var(--hover); }
.item:focus-visible { outline: none; box-shadow: 0 0 0 3px rgba(10, 132, 255, 0.42); }
.item span:first-of-type { flex: 1; }
.item-icon { color: var(--label); }
.kbd { font-size: 12px; color: var(--muted); }
@media (prefers-reduced-motion: reduce) { .panel { animation: fade 150ms ease both; } }
@keyframes fade { from { opacity: 0; } to { opacity: 1; } }
</style>
