<script setup lang="ts">
import { onMounted, ref } from 'vue'
import Icon from '../components/Icon.vue'
import { masked, serverInfo, type ServerInfo } from '../lib/server'

const info = ref<ServerInfo | null>(null)
const copied = ref(false)

onMounted(async () => {
  info.value = await serverInfo()
})

async function copyUrl() {
  if (!info.value) return
  await navigator.clipboard.writeText(info.value.url)
  copied.value = true
  setTimeout(() => (copied.value = false), 1600)
}
</script>

<template>
  <div class="page">
    <section class="group card" aria-labelledby="server-title">
      <div class="head">
        <Icon name="server" :size="18" class="head-icon" />
        <h2 id="server-title">Lokale MCP-server</h2>
        <span v-if="info" class="state" :class="info.running ? 'ok' : 'bad'">
          <span class="dot" :class="info.running ? 'on' : 'bad'" />{{ info.running ? 'Actief' : 'Uit' }}
        </span>
      </div>
      <p v-if="info?.error" class="error rise" role="alert">{{ info.error }}</p>
      <dl v-if="info" class="facts">
        <dt>Adres</dt>
        <dd class="mono selectable">{{ info.url }}</dd>
        <dd class="actions">
          <button type="button" class="btn small ghost" @click="copyUrl">
            <Icon name="copy" :size="14" />{{ copied ? 'Gekopieerd' : 'Kopiëren' }}
          </button>
        </dd>
        <dt>Token</dt>
        <dd class="mono">{{ masked(info.tokenTail) }}</dd>
        <dd />
      </dl>
      <div class="chips">
        <span class="chip">Alleen 127.0.0.1</span>
        <span class="chip">Websites geweigerd (Origin)</span>
        <span class="chip">Alleen lezen</span>
      </div>
    </section>
  </div>
</template>

<style scoped>
.page { padding: 8px 28px 28px; display: flex; flex-direction: column; gap: 16px; }
.card { padding: 18px 20px; display: flex; flex-direction: column; gap: 14px; }
.head { display: flex; align-items: center; gap: 10px; }
.head h2 { flex: 1; font-size: 13px; font-weight: 700; }
.head-icon { color: var(--label); }
.state { display: inline-flex; align-items: center; gap: 6px; height: 20px; padding: 0 8px; border-radius: 999px; font-size: 11px; font-weight: 500; }
.state.ok { background: var(--success-soft); color: var(--success); }
.state.bad { background: var(--danger-soft); color: var(--danger); }
.error { font-size: 12px; color: var(--danger); }
.facts { display: grid; grid-template-columns: 90px minmax(0, 1fr) auto; gap: 8px 14px; align-items: center; margin: 0; }
.facts dt { color: var(--muted); }
.facts dd { margin: 0; }
.actions { display: flex; }
.chips { display: flex; flex-wrap: wrap; gap: 8px; }
.chip { height: 18px; display: inline-flex; align-items: center; padding: 0 7px; border-radius: 999px; background: var(--subtle); color: var(--label); font-size: 11px; font-weight: 500; }
</style>
