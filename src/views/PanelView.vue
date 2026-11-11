<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import Alert from '../components/Alert.vue'
import Avatar from '../components/Avatar.vue'
import Icon from '../components/Icon.vue'
import SigninSteps from '../components/SigninSteps.vue'
import {
  askedCode, cancelSignIn, connect, dismissSignin, isRunning, sessionAge, signIn, signingIn, signOut, signOutAll, startedHere, store,
  type AccountView,
} from '../lib/accounts'
import { fitPanel, isMac, modKey, openMain, quitApp, runningCalls, server, watchServer } from '../lib/server'
import { checkNow } from '../lib/updates'

const info = computed(() => server.info)
const now = ref(Date.now())
const quitting = ref(false)

// The panel loads at launch, before a port error is known: the server state follows events, focus and a timer.
let stopServer: (() => void) | null = null
let clockTimer: ReturnType<typeof setInterval> | undefined
onMounted(async () => {
  stopServer = watchServer()
  clockTimer = setInterval(() => (now.value = Date.now()), 30_000)
  await connect()
})
onBeforeUnmount(() => {
  stopServer?.()
  clearInterval(clockTimer)
})

const active = computed(() => store.accounts.filter((a) => a.session && !a.session.expired))
const subtitle = computed(() => {
  const n = active.value.length
  if (!n) return 'Niet ingelogd'
  return n === 1 ? `1 sessie · ${active.value[0]!.label}` : `${n} sessies`
})

const items = [
  { icon: 'chart', label: 'Dashboard', route: '/dashboard' },
  { icon: 'list', label: 'Verzoeken', route: '/verzoeken' },
  { icon: 'users', label: 'Accounts beheren', route: '/accounts' },
  { icon: 'plug', label: 'Koppelen met een AI-client', route: '/koppelen' },
]

function line(account: AccountView): string {
  if (account.session?.expired) return 'Sessie verlopen'
  if (account.session) return account.session.administration ?? account.administration ?? account.username
  return account.store === 'onePassword' ? `${account.username || account.label} · 1Password` : `${account.username} · Sleutelhanger`
}

function sessionLine(account: AccountView): string | null {
  const s = account.session
  if (!s || s.expired) return null
  return sessionAge(s.startedAt, now.value)
}

/** Running here or elsewhere: a sign-in started in the main window (or before this window loaded) counts too. */
function busy(account: AccountView): boolean {
  return signingIn(account)
}

/**
 * The steps open only for a sign-in started in this panel, or one that needs a code, wherever it started, and
 * stay open with their end state and diagnosis until Sluiten. Other windows' sign-ins show as a busy row.
 */
function showSteps(account: AccountView): boolean {
  if (!store.signins[account.id]) return false
  return startedHere(account.id) || askedCode(account.id)
}

/** macOS: Sparkle's own window. Windows: the main window shows the update card after a fresh check. */
async function checkForUpdates() {
  await checkNow()
  if (!isMac) await openMain('/dashboard')
}

/** Asks first only while an AI client is waiting on a call; otherwise quits at once. */
const confirmQuit = ref(0)
async function quit() {
  if (quitting.value) return
  const running = await runningCalls()
  if (running > 0 && !confirmQuit.value) {
    confirmQuit.value = running
    return
  }
  confirmQuit.value = 0
  quitting.value = true
  await quitApp()
}

function onKey(event: KeyboardEvent) {
  if (event.key.toLowerCase() === 'q' && (event.metaKey || event.ctrlKey)) {
    event.preventDefault()
    void quit()
  }
}
onMounted(() => window.addEventListener('keydown', onKey))

// The window takes the height of what it shows, so there is no empty glass under Afsluiten.
// The quit alert needs room of its own while it is open.
const content = ref<HTMLElement | null>(null)
let frame = 0
const fit = () => {
  cancelAnimationFrame(frame)
  frame = requestAnimationFrame(() => {
    if (!content.value) return
    const height = Math.ceil(content.value.getBoundingClientRect().height) + 6
    void fitPanel(confirmQuit.value ? Math.max(height, 280) : height)
  })
}
const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(fit)
onMounted(() => content.value && observer?.observe(content.value))
watch(confirmQuit, fit)
onBeforeUnmount(() => observer?.disconnect())
onBeforeUnmount(() => window.removeEventListener('keydown', onKey))
</script>

<template>
  <div class="panel" role="dialog" aria-label="EB MCP">
    <div ref="content" class="panel-content">
    <div class="head">
      <img src="/app-icon.png" alt="" width="30" height="30" />
      <span class="name">
        <strong>EB MCP</strong>
        <span>{{ subtitle }}</span>
      </span>
      <span class="state" :class="info?.running ? 'ok' : 'wait'">
        <span class="dot" :class="info?.running ? 'on' : 'off'" />{{ info?.running ? 'MCP actief' : 'MCP uit' }}
      </span>
    </div>

    <div class="section-label label">Accounts</div>
    <p v-if="store.loaded && !store.accounts.length" class="hint">
      Nog geen accounts. Voeg er een toe via Accounts beheren.
    </p>
    <div class="accounts">
      <div v-for="account in store.accounts" :key="account.id" class="account" :class="{ expired: account.session?.expired, busy: busy(account) }">
        <div class="account-row">
          <Avatar :label="account.label" :size="30" :muted="!account.session || account.session.expired" />
          <span class="stack">
            <span class="title">{{ account.label }}</span>
            <span class="sub" :class="{ bad: account.session?.expired }">{{ line(account) }}</span>
            <span v-if="sessionLine(account)" class="sub">{{ sessionLine(account) }}</span>
          </span>
          <button v-if="busy(account)" type="button" class="btn small ghost" @click="cancelSignIn(account.id)">Annuleren</button>
          <button v-else-if="account.session && !account.session.expired" type="button" class="btn small" @click="signOut(account.id)">Uitloggen</button>
          <button v-else type="button" class="btn small" :class="{ primary: account.session?.expired }" @click="signIn(account.id)">
            {{ account.session?.expired ? 'Opnieuw inloggen' : 'Inloggen' }}
          </button>
        </div>
        <Transition name="accordion">
          <div v-if="showSteps(account)" class="accordion">
            <div>
              <div class="inline">
                <SigninSteps :account="account" :event="store.signins[account.id]" compact />
                <button v-if="!isRunning(store.signins[account.id]?.step)" type="button" class="btn small ghost close" @click="dismissSignin(account.id)">Sluiten</button>
              </div>
            </div>
          </div>
        </Transition>
      </div>
    </div>
    <p v-if="!active.length && store.accounts.length" class="hint">AI-clients horen nu dat ze eerst moeten wachten tot je inlogt.</p>

    <div class="sep" />
    <button v-for="item in items" :key="item.route" type="button" class="item" @click="openMain(item.route)">
      <Icon :name="item.icon" :size="15" class="item-icon" />
      <span>{{ item.label }}</span>
    </button>
    <button type="button" class="item" @click="checkForUpdates">
      <Icon name="refresh" :size="15" class="item-icon" />
      <span>Zoek naar updates…</span>
    </button>
    <div class="sep" />
    <button v-if="active.length > 1" type="button" class="item" @click="signOutAll">
      <Icon name="log-out" :size="15" class="item-icon" />
      <span>Alles uitloggen</span>
    </button>
    <button type="button" class="item" :disabled="quitting" :aria-busy="quitting" @click="quit">
      <span class="swap item-icon">
        <Icon name="power" :size="15" :class="{ off: quitting }" />
        <span class="spinner" :class="{ off: !quitting }" />
      </span>
      <span class="swap">
        <span :class="{ off: quitting }" :aria-hidden="quitting">Afsluiten</span>
        <span :class="{ off: !quitting }" :aria-hidden="!quitting">Afsluiten…</span>
      </span>
      <span class="kbd">{{ modKey }}Q</span>
    </button>
    </div>
    <Transition name="modal">
      <Alert v-if="confirmQuit" title="Toch afsluiten?" tone="warn" @close="confirmQuit = 0">
        <p>{{ confirmQuit === 1 ? 'Er loopt nog 1 aanroep' : `Er lopen nog ${confirmQuit} aanroepen` }} van een AI-client. Die krijgt dan een fout, en EB MCP vergeet alle sessies.</p>
        <template #buttons>
          <button type="button" class="btn" @click="confirmQuit = 0">Annuleren</button>
          <button type="button" class="btn danger-fill" @click="quit">Afsluiten</button>
        </template>
      </Alert>
    </Transition>
  </div>
</template>

<style scoped>
.panel { height: 100%; box-sizing: border-box; padding: 0 6px 6px; overflow-y: auto; }
.panel-content { display: flex; flex-direction: column; }
.head { display: flex; align-items: center; gap: 10px; padding: 12px 8px 8px; }
.head img { border-radius: 7px; }
.name { display: flex; flex-direction: column; flex: 1; min-width: 0; }
.name span { font-size: 11px; color: var(--muted); }
.state { display: inline-flex; align-items: center; gap: 5px; height: 20px; padding: 0 8px; border-radius: 999px; font-size: 11px; font-weight: 500; }
.state.ok { background: var(--success-soft); color: var(--success); }
.state.wait { background: var(--subtle); color: var(--label); }
.label { padding: 6px 10px 3px; }
.hint { padding: 2px 10px 6px; font-size: 11px; line-height: 15px; color: var(--muted); }
.accounts { display: flex; flex-direction: column; gap: 1px; }
.account { border-radius: 10px; transition: background-color 150ms ease; }
.account:hover, .account.busy { background: var(--hover); }
.account.expired { background: var(--danger-soft); }
.account-row { display: flex; align-items: center; gap: 10px; padding: 6px 8px; }
.stack { display: flex; flex-direction: column; gap: 1px; flex: 1; min-width: 0; }
.title { display: flex; align-items: center; gap: 6px; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.sub { font-size: 11px; color: var(--muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.sub.bad { color: var(--danger); }
.inline { padding: 0 10px 10px 48px; display: flex; flex-direction: column; gap: 6px; }
.close { align-self: flex-end; }
.sep { height: 1px; background: var(--line-soft); margin: 5px 10px; flex: none; }
.item {
  box-sizing: border-box; width: 100%; min-height: 26px; display: flex; align-items: center; gap: 8px; padding: 0 8px; flex: none;
  border: 0; border-radius: 8px; background: transparent; text-align: left; transition: background-color 120ms ease;
}
.item:hover { background: var(--hover); }
.item:focus-visible { outline: none; box-shadow: 0 0 0 3px rgba(10, 132, 255, 0.42); }
.item > .item-icon + span { flex: 1; }
.item-icon { color: var(--label); }
.swap.item-icon > * { align-self: center; justify-self: center; }
.spinner { box-sizing: border-box; width: 12px; height: 12px; border-radius: 50%; border: 1.5px solid var(--line); border-top-color: var(--label); }
.item[aria-busy='true'] .spinner { animation: spin 800ms linear infinite; }
.kbd { font-size: 12px; color: var(--muted); }
@media (prefers-reduced-motion: reduce) { .item[aria-busy='true'] .spinner { animation-duration: 2400ms; } }
</style>
