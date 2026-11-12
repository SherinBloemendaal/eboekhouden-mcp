<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import Alert from './Alert.vue'
import { store } from '../lib/accounts'
import { approvalState, connectApprovals, quitWarning, waitingText, type Proposal } from '../lib/approvals'
import { hostPort, inApp, modKey, quitApp, server, startupNotice, watchServer } from '../lib/server'
import {
  clampWidth, isSidebarShortcut, loadSidebar, saveSidebar, setSidebarCollapsed, setSidebarWidth, sidebar, sidebarShortcut, sidebarShortcutAria,
  SIDEBAR_DEFAULT, SIDEBAR_MAX, SIDEBAR_MIN, SIDEBAR_STEP,
} from '../lib/sidebar'
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

// The width and the collapsed state the user left, read before the first render so the sidebar never jumps.
loadSidebar()

// Cmd+, (Ctrl+, on Windows) opens Instellingen, as in every Mac app.
function onKey(event: KeyboardEvent) {
  if (event.key === ',' && (event.metaKey || event.ctrlKey)) {
    event.preventDefault()
    void router.push('/instellingen')
    return
  }
  if (isSidebarShortcut(event)) {
    event.preventDefault()
    // Not behind an open dialog: it would hand the focus back to a sidebar that is no longer there.
    if (!document.querySelector('[aria-modal="true"]')) void toggleSidebar()
  }
}

// Collapsing: the sidebar's button moves to the toolbar. Whoever had the focus on one button (or in the sidebar)
// gets it on the other, so the focus never stays behind in a sidebar that is out of reach.
const sideEl = ref<HTMLElement | null>(null)
const sideToggle = ref<HTMLButtonElement | null>(null)
const barToggle = ref<HTMLButtonElement | null>(null)
const toggleLabel = computed(() => (sidebar.collapsed ? 'Zijbalk tonen' : 'Zijbalk verbergen'))
// The width animates only while the sidebar opens or closes; dragging and the arrow keys follow the hand at once.
const toggling = ref(false)
let toggleTimer: ReturnType<typeof setTimeout> | undefined
async function toggleSidebar() {
  const active = document.activeElement
  const moveFocus = active === sideToggle.value || active === barToggle.value || (active instanceof Node && !!sideEl.value?.contains(active))
  toggling.value = true
  clearTimeout(toggleTimer)
  toggleTimer = setTimeout(() => (toggling.value = false), 300)
  setSidebarCollapsed(!sidebar.collapsed)
  if (!moveFocus) return
  await nextTick()
  ;(sidebar.collapsed ? barToggle.value : sideToggle.value)?.focus()
}

// The handle on the sidebar's right edge: drag it, or use the arrow keys, Home and End; a double click resets it.
const resizing = ref(false)
let drag: { pointer: number; startX: number; startWidth: number } | null = null
function onHandleDown(event: PointerEvent) {
  if (event.button !== 0 || sidebar.collapsed) return
  // No text selection and no focus ring from the press itself.
  event.preventDefault()
  const handle = event.currentTarget as HTMLElement
  handle.setPointerCapture?.(event.pointerId)
  drag = { pointer: event.pointerId, startX: event.clientX, startWidth: sidebar.width }
  resizing.value = true
}
function onHandleMove(event: PointerEvent) {
  if (!drag || event.pointerId !== drag.pointer) return
  sidebar.width = clampWidth(drag.startWidth + event.clientX - drag.startX)
}
function onHandleUp(event: PointerEvent) {
  if (!drag || event.pointerId !== drag.pointer) return
  drag = null
  resizing.value = false
  const handle = event.currentTarget as HTMLElement
  if (handle.hasPointerCapture?.(event.pointerId)) handle.releasePointerCapture(event.pointerId)
  saveSidebar()
}
function onHandleKey(event: KeyboardEvent) {
  const next: Record<string, number> = {
    ArrowLeft: sidebar.width - SIDEBAR_STEP, ArrowRight: sidebar.width + SIDEBAR_STEP, Home: SIDEBAR_MIN, End: SIDEBAR_MAX,
  }
  const width = next[event.key]
  if (width === undefined) return
  event.preventDefault()
  setSidebarWidth(width)
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
  clearTimeout(toggleTimer)
  unlistenQuit?.()
  stopServer?.()
})
</script>

<template>
  <div
    class="shell" :class="{ collapsed: sidebar.collapsed, toggling, resizing }"
    :style="{ '--side-w': `${sidebar.width}px` }"
  >
    <!-- Collapsed, the sidebar is inert: its links leave the tab order and assistive tech skips them. Open, the attribute
         is left out (undefined), never written as inert="false", which would still count as inert. -->
    <nav id="app-sidebar" ref="sideEl" class="sidebar" aria-label="Hoofdmenu" :inert="sidebar.collapsed || undefined" data-tauri-drag-region>
      <!-- The inside keeps the full width while the sidebar opens or closes, so nothing in it wraps or shifts. -->
      <div class="side-inner" data-tauri-drag-region>
        <div class="traffic-space" data-tauri-drag-region>
          <button
            ref="sideToggle" type="button" class="side-toggle" aria-controls="app-sidebar" :aria-expanded="!sidebar.collapsed"
            :aria-label="toggleLabel" :aria-keyshortcuts="sidebarShortcutAria" :title="`${toggleLabel} (${sidebarShortcut})`"
            @click="toggleSidebar"
          >
            <Icon name="sidebar-left" :size="18" />
          </button>
        </div>
        <div class="section-label side-label" data-tauri-drag-region>EB MCP</div>
        <RouterLink
          v-for="item in nav" :key="item.to" :to="item.to" class="nav" active-class="active"
          :aria-label="item.to === '/akkoord' && waiting ? `${item.label}, ${waitingText(waiting)}` : undefined"
        >
          <Icon :name="item.icon" class="nav-icon" />
          <span class="nav-label">{{ item.label }}</span>
          <Transition v-if="item.to === '/akkoord'" name="pop">
            <span v-if="waiting" class="nav-badge" aria-hidden="true">{{ waiting }}</span>
          </Transition>
        </RouterLink>
        <div class="side-foot">
          <UpdateUi />
          <RouterLink to="/instellingen" class="nav settings" active-class="active">
            <Icon name="settings" class="nav-icon" />
            <span class="nav-label">Instellingen</span>
            <span class="shortcut">{{ modKey }},</span>
          </RouterLink>
          <span class="server-line">
            <span class="dot" :class="info?.running ? 'on' : 'bad'" />
            <span class="server-state">{{ info?.running ? 'MCP actief' : 'MCP staat uit' }}</span>
          </span>
          <span class="mono server-addr">{{ info?.error ?? (address || '\u00a0') }}</span>
        </div>
      </div>
    </nav>
    <div
      class="side-handle" :class="{ 'at-min': sidebar.width <= SIDEBAR_MIN, 'at-max': sidebar.width >= SIDEBAR_MAX }"
      role="separator" aria-orientation="vertical" aria-label="Breedte van de zijbalk" aria-controls="app-sidebar"
      :aria-valuemin="SIDEBAR_MIN" :aria-valuemax="SIDEBAR_MAX" :aria-valuenow="sidebar.width"
      :aria-valuetext="`${sidebar.width} pixels breed`" :tabindex="sidebar.collapsed ? -1 : 0" :inert="sidebar.collapsed || undefined"
      @pointerdown="onHandleDown" @pointermove="onHandleMove" @pointerup="onHandleUp" @pointercancel="onHandleUp"
      @lostpointercapture="onHandleUp" @dblclick="setSidebarWidth(SIDEBAR_DEFAULT)" @keydown="onHandleKey"
    />
    <main class="main">
      <!-- deep: the title drags too; buttons in the toolbar stay clickable. -->
      <header class="toolbar" data-tauri-drag-region="deep">
        <!-- Collapsed, the sidebar's button waits here, clear of the window buttons; the title moves aside for it. -->
        <Transition name="fade">
          <button
            v-if="sidebar.collapsed" ref="barToggle" type="button" class="side-toggle bar-toggle" aria-controls="app-sidebar"
            :aria-expanded="!sidebar.collapsed" :aria-label="toggleLabel" :aria-keyshortcuts="sidebarShortcutAria"
            :title="`${toggleLabel} (${sidebarShortcut})`" @click="toggleSidebar"
          >
            <Icon name="sidebar-left" :size="18" />
          </button>
        </Transition>
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
.shell {
  --side-card: calc(var(--side-w) - 18px);
  /* Where the toolbar's sidebar button starts when the sidebar is collapsed: clear of the three window buttons
     (trafficLightPosition x 22 in tauri.conf.json). Windows has its window buttons in the title bar: windows.css. */
  --toolbar-start: 92px;
  display: flex; height: 100%; background: var(--glass-window);
}
/* A floating glass sidebar, inset from the window edge, with the traffic lights inside it (the boards' window()).
   Its width is the user's (--side-w, from lib/sidebar.ts); collapsed it narrows to nothing and fades. */
.sidebar {
  /* Measured on a macOS 26 window (Chatkeep, 2026-11-12): the sidebar's column is --side-w in all. The card
     sits 8 from the window's edges and leaves 10 to the content, so it is 18 narrower than the column; its
     corners are 18, concentric with the window's 26. */
  width: var(--side-card); flex: none; box-sizing: border-box; overflow: hidden;
  margin: 8px 0 8px 8px; border-radius: 18px;
  background: var(--glass-side); box-shadow: var(--glass-side-edge);
  -webkit-backdrop-filter: blur(30px) saturate(180%); backdrop-filter: blur(30px) saturate(180%);
  transition: margin-left 220ms var(--ease), opacity 220ms var(--ease);
}
.shell.toggling .sidebar { transition: width 220ms var(--ease), margin-left 220ms var(--ease), opacity 220ms var(--ease); }
.shell.collapsed .sidebar { width: 0; margin-left: 0; opacity: 0; }
.side-inner {
  width: var(--side-card); height: 100%; box-sizing: border-box; display: flex; flex-direction: column; padding: 0 10px 10px;
}
/* 44 high, so the first row starts 52 from the window's top; the button is centred 26 from it, on the line of
   the window buttons. */
.traffic-space { box-sizing: border-box; height: 44px; padding-bottom: 8px; flex: none; display: flex; align-items: center; justify-content: flex-end; }
/* The macOS sidebar button: the glyph alone, a soft fill on hover. */
.side-toggle {
  box-sizing: border-box; width: 28px; height: 28px; flex: none; display: inline-flex; align-items: center; justify-content: center;
  padding: 0; border: 0; border-radius: 999px; background: transparent; color: var(--muted);
  transition: background-color 150ms ease, color 150ms ease, box-shadow 150ms ease;
}
.side-toggle:hover { background: var(--hover); color: var(--text); }
.side-toggle:focus-visible { outline: none; box-shadow: 0 0 0 3.5px rgba(10, 132, 255, 0.42); }
.side-label { padding: 6px 8px 4px; }
.nav {
  box-sizing: border-box; height: 32px; display: flex; align-items: center; gap: 8px; padding: 0 8px;
  border-radius: 8px; color: var(--text); text-decoration: none; transition: background-color 150ms ease, color 150ms ease;
}
.nav:hover { background: var(--hover); }
.nav.active { background: var(--selected); }
.nav-icon { color: var(--accent); transition: color 150ms ease; }
/* As a macOS list does it: the selected row takes the system accent with white text while the sidebar has the
   focus, and the grey fill above when the content has it. --accent is the Mac's own accent (tokens.css). */
.sidebar:focus-within .nav.active { background: var(--accent); color: #fff; }
.sidebar:focus-within .nav.active .nav-icon,
.sidebar:focus-within .nav.active .shortcut { color: #fff; }
.sidebar:focus-within .nav.active .nav-badge { background: #fff; color: var(--accent); }
/* A narrow sidebar shortens the label, never the badge or the shortcut beside it. */
.nav-label { flex: 1 1 auto; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.settings { margin: 0 -4px 4px; }
.shortcut { flex: none; margin-left: auto; font-size: 11px; color: var(--muted); }
.side-foot { margin-top: auto; display: flex; flex-direction: column; gap: 6px; padding: 0 4px; min-width: 0; }
.server-line { display: flex; align-items: center; gap: 7px; font-size: 12px; font-weight: 500; min-width: 0; }
.server-state { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
/* An error from the server wraps rather than being cut: it is the only place it shows. */
.server-addr { font-size: 11px; color: var(--muted); padding-left: 14px; overflow-wrap: anywhere; }
/* The handle straddles the sidebar's right edge without taking room; a thin line fades in on hover. */
.side-handle {
  position: relative; z-index: 2; flex: none; width: 10px; margin: 8px -5px; cursor: col-resize; outline: none; touch-action: none;
}
.side-handle.at-min { cursor: e-resize; }
.side-handle.at-max { cursor: w-resize; }
.side-handle::after {
  content: ''; position: absolute; top: 14px; bottom: 14px; left: 4px; width: 2px; border-radius: 1px;
  background: var(--line); opacity: 0; transition: opacity 150ms ease, background-color 150ms ease;
}
.side-handle:hover::after, .shell.resizing .side-handle::after { opacity: 1; }
.side-handle:focus-visible::after { opacity: 1; background: var(--accent); }
.shell.collapsed .side-handle { visibility: hidden; }
/* While dragging: the resize cursor everywhere and no text selection, also over the content. */
.shell.resizing, .shell.resizing :deep(*) { cursor: col-resize; -webkit-user-select: none; user-select: none; }
.main { flex: 1; min-width: 0; display: flex; flex-direction: column; }
.toolbar {
  position: relative; height: 52px; flex: none; display: flex; align-items: center; padding: 0 20px;
  transition: padding-left 220ms var(--ease);
}
.shell.collapsed .toolbar { padding-left: calc(var(--toolbar-start) + 28px + 10px); }
.bar-toggle { position: absolute; left: var(--toolbar-start); top: 50%; margin-top: -14px; }
.toolbar h1 { font-size: 15px; font-weight: 700; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
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
  font-variant-numeric: tabular-nums; flex: none;
}
@media (prefers-reduced-motion: reduce) {
  .sidebar, .shell.toggling .sidebar, .toolbar { transition: none; }
}
</style>
