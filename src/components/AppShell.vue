<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import Alert from './Alert.vue'
import { store } from '../lib/accounts'
import { approvalState, connectApprovals, quitWarning, waitingText, type Proposal } from '../lib/approvals'
import { hostPort, inApp, modKey, quitApp, server, startupNotice, watchServer } from '../lib/server'
import Icon from './Icon.vue'
import ToastStack, { type Toast } from './ToastStack.vue'
import UpdateUi from './UpdateUi.vue'

const nav = [
  { to: '/dashboard', icon: 'chart', label: 'Dashboard' },
  { to: '/accounts', icon: 'users', label: 'Accounts' },
  { to: '/akkoord', icon: 'clipboard-check', label: 'Akkoord' },
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
const quitAsked = ref(false)
const quitting = ref(false)
let unlistenQuit: (() => void) | null = null
let unmounted = false
onMounted(async () => {
  if (!inApp()) return
  const { listen } = await import('@tauri-apps/api/event')
  const stop = await listen<number>('quit-requested', (event) => {
    quitCalls.value = event.payload
    quitAsked.value = true
  })
  if (unmounted) stop()
  else unlistenQuit = stop
})
async function confirmQuit() {
  quitting.value = true
  await quitApp()
}
const title = computed(() => String(route.meta.title ?? ''))

// The server can fail after this window loaded (a port in use): follow it instead of reading it once.
const info = computed(() => server.info)
const address = computed(() => (info.value ? hostPort(info.value.url) : ''))
let stopServer: (() => void) | null = null
onMounted(() => {
  stopServer = watchServer()
})

// Something EB MCP noticed at startup (a damaged accounts.json it set aside): a banner until it is closed.
const notice = ref<string | null>(null)
onMounted(async () => {
  try {
    notice.value = await startupNotice()
  } catch (error) {
    console.error('startup_notice', error)
  }
})

// Changes waiting for approval: a badge on Akkoord, a banner on every other page, and a toast for each new one.
const waiting = computed(() => store.approvals.length)
const onApprovals = computed(() => route.path === '/akkoord')
onMounted(() => void connectApprovals())

const toasts = ref<Toast[]>([])
/** Proposals this window already knew: the ones in the first read after this shell mounted are not news. */
const seen = new Set<number>()
let primed = false

function toast(proposal: Proposal): Toast {
  return { id: proposal.id, title: proposal.title, text: `Voorstel ${proposal.id} · ${proposal.account} · ${proposal.client}`, action: 'Bekijken', icon: 'clipboard-check' }
}

// Each read of the queue (connectApprovals above starts one) counts; a list read before this shell mounted does not.
watch(
  () => approvalState.revision,
  () => {
    const list = store.approvals
    const fresh = list.filter((p) => !seen.has(p.id))
    for (const p of fresh) seen.add(p.id)
    if (!primed) {
      primed = true
      return
    }
    // A proposal answered elsewhere takes its toast with it.
    const ids = new Set(list.map((p) => p.id))
    const kept = toasts.value.filter((t) => ids.has(t.id as number))
    // Newest on top; on Akkoord itself the list shows them already.
    toasts.value = onApprovals.value ? kept : [...fresh.reverse().map(toast), ...kept]
  },
)
watch(onApprovals, (here) => {
  if (here) toasts.value = []
})

function closeToast(id: Toast['id']) {
  toasts.value = toasts.value.filter((t) => t.id !== id)
}

function openToast(id: Toast['id']) {
  void router.push({ path: '/akkoord', query: { open: String(id) } })
}

onBeforeUnmount(() => {
  unmounted = true
  window.removeEventListener('keydown', onKey)
  unlistenQuit?.()
  stopServer?.()
})
</script>

<template>
  <div class="shell">
    <nav class="sidebar" aria-label="Hoofdmenu" data-tauri-drag-region>
      <div class="traffic-space" data-tauri-drag-region />
      <div class="section-label side-label" data-tauri-drag-region>EB MCP</div>
      <RouterLink
        v-for="item in nav" :key="item.to" :to="item.to" class="nav" active-class="active"
        :aria-label="item.to === '/akkoord' && waiting ? `${item.label}, ${waitingText(waiting)}` : undefined"
      >
        <Icon :name="item.icon" class="nav-icon" />
        <span>{{ item.label }}</span>
        <Transition v-if="item.to === '/akkoord'" name="pop">
          <span v-if="waiting" class="nav-badge" aria-hidden="true">{{ waiting }}</span>
        </Transition>
      </RouterLink>
      <div class="side-foot">
        <UpdateUi />
        <RouterLink to="/instellingen" class="nav settings" active-class="active">
          <Icon name="settings" class="nav-icon" />
          <span>Instellingen</span>
          <span class="shortcut">{{ modKey }},</span>
        </RouterLink>
        <span class="server-line">
          <span class="dot" :class="info?.running ? 'on' : 'bad'" />
          {{ info?.running ? 'MCP actief' : 'MCP staat uit' }}
        </span>
        <span class="mono server-addr">{{ info?.error ?? (address || '\u00a0') }}</span>
      </div>
    </nav>
    <main class="main">
      <!-- deep: the title drags too; buttons in the toolbar stay clickable. -->
      <header class="toolbar" data-tauri-drag-region="deep">
        <h1>{{ title }}</h1>
        <span id="toolbar-actions" class="toolbar-actions" />
      </header>
      <Transition name="accordion">
        <div v-if="notice" class="accordion">
          <div>
            <div class="notice" role="status">
              <Icon name="alert" :size="15" class="notice-icon" />
              <p class="notice-text selectable">{{ notice }}</p>
              <button type="button" class="btn small ghost" @click="notice = null">Sluiten</button>
            </div>
          </div>
        </div>
      </Transition>
      <Transition name="accordion">
        <div v-if="waiting && !onApprovals" class="accordion">
          <div>
            <div class="notice info approval-notice" role="status">
              <Icon name="clipboard-check" :size="15" class="notice-icon" />
              <p class="notice-text">{{ waitingText(waiting) }}</p>
              <button type="button" class="btn small ghost" @click="router.push('/akkoord')">Bekijken</button>
            </div>
          </div>
        </div>
      </Transition>
      <!-- The toasts sit at the top right of the page, under the toolbar and any banner. -->
      <div class="content-area">
        <div class="content">
          <RouterView />
        </div>
        <ToastStack :toasts="toasts" label="Wijzigingen die wachten" @close="closeToast" @action="openToast" />
      </div>
    </main>
  </div>
  <Transition name="modal">
    <Alert v-if="quitAsked" title="Toch afsluiten?" tone="warn" @close="quitAsked = false">
      <p>{{ quitWarning(quitCalls, store.approvals.length) }}</p>
      <template #buttons>
        <button type="button" class="btn" @click="quitAsked = false">Annuleren</button>
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
.content-area { flex: 1; min-height: 0; position: relative; display: flex; flex-direction: column; }
.content { flex: 1; min-height: 0; overflow: auto; }
/* The margin sits inside the collapsing block, so it opens and closes with it. */
.notice {
  display: flex; align-items: flex-start; gap: 10px; margin: 0 20px 12px; padding: 9px 8px 9px 12px;
  border-radius: 10px; background: var(--warn-soft); color: var(--warn); font-size: 12px; line-height: 16px;
}
.notice-icon { flex: none; margin-top: 4px; }
.notice-text { flex: 1; min-width: 0; padding-top: 4px; overflow-wrap: anywhere; }
.notice .btn { flex: none; color: var(--warn); }
.notice.info { background: var(--info-soft); color: var(--info); }
.notice.info .btn { color: var(--info); }
.nav-badge {
  margin-left: auto; box-sizing: border-box; min-width: 18px; height: 18px; padding: 0 5px; border-radius: 999px;
  background: var(--accent); color: #fff; font-size: 11px; font-weight: 600; line-height: 18px; text-align: center;
  font-variant-numeric: tabular-nums;
}
</style>
