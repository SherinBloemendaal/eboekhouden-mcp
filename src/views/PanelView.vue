<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import Avatar from '../components/Avatar.vue'
import Icon from '../components/Icon.vue'
import SigninSteps from '../components/SigninSteps.vue'
import { cancelSignIn, clock, connect, dismissSignin, isRunning, signIn, signOut, signOutAll, since, store, type AccountView } from '../lib/accounts'
import { openMain, quitApp, serverInfo, type ServerInfo } from '../lib/server'
import { checkNow } from '../lib/updates'

const info = ref<ServerInfo | null>(null)
const now = ref(Date.now())
const quitting = ref(false)

onMounted(async () => {
  await connect()
  info.value = await serverInfo()
  setInterval(() => (now.value = Date.now()), 30_000)
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
  { icon: 'plug', label: 'Koppelen met Claude of Cursor', route: '/koppelen' },
]

function line(account: AccountView): string {
  if (account.session?.expired) return 'Sessie verlopen'
  if (account.session) return account.session.administration ?? account.administration ?? account.username
  return account.store === 'onePassword' ? `${account.username || account.label} · 1Password` : `${account.username} · Sleutelhanger`
}

function sessionLine(account: AccountView): string | null {
  const s = account.session
  if (!s || s.expired) return null
  return `Ingelogd ${since(s.startedAt, now.value)}${s.renewedAt ? ` · verlengd ${clock(s.renewedAt)}` : ''}`
}

/** macOS: Sparkle's own window. Windows: the main window shows the update card after a fresh check. */
async function checkForUpdates() {
  await checkNow()
  if (!navigator.userAgent.includes('Mac')) await openMain('/dashboard')
}

async function quit() {
  quitting.value = true
  await quitApp()
}
</script>

<template>
  <div class="panel" role="dialog" aria-label="EB MCP">
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
      <div v-for="account in store.accounts" :key="account.id" class="account" :class="{ expired: account.session?.expired, busy: isRunning(store.signins[account.id]?.step) }">
        <div class="account-row">
          <Avatar :label="account.label" :size="30" :muted="!account.session || account.session.expired" />
          <span class="stack">
            <span class="title">{{ account.label }}<span v-if="account.isDefault" class="chip">Standaard</span></span>
            <span class="sub" :class="{ bad: account.session?.expired }">{{ line(account) }}</span>
            <span v-if="sessionLine(account)" class="sub">{{ sessionLine(account) }}</span>
          </span>
          <button v-if="isRunning(store.signins[account.id]?.step)" type="button" class="btn small ghost" @click="cancelSignIn(account.id)">Annuleren</button>
          <button v-else-if="account.session && !account.session.expired" type="button" class="btn small" @click="signOut(account.id)">Uitloggen</button>
          <button v-else type="button" class="btn small" :class="{ primary: account.session?.expired || account.isDefault }" @click="signIn(account.id)">
            {{ account.session?.expired ? 'Opnieuw inloggen' : 'Inloggen' }}
          </button>
        </div>
        <Transition name="accordion">
          <div v-if="store.signins[account.id]" class="accordion">
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
        <span :class="{ off: !quitting }" :aria-hidden="!quitting">Uitloggen en afsluiten…</span>
      </span>
      <span class="kbd">⌘Q</span>
    </button>
  </div>
</template>

<style scoped>
.panel { height: 100%; box-sizing: border-box; padding: 0 6px 6px; display: flex; flex-direction: column; overflow-y: auto; }
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
.chip { height: 16px; display: inline-flex; align-items: center; padding: 0 6px; border-radius: 999px; background: var(--info-soft); color: var(--info); font-size: 10px; font-weight: 500; }
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
