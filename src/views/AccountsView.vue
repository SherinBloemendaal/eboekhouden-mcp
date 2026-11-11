<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import AccountForm from '../components/AccountForm.vue'
import Alert from '../components/Alert.vue'
import Avatar from '../components/Avatar.vue'
import EmptyState from '../components/EmptyState.vue'
import Icon from '../components/Icon.vue'
import Sheet from '../components/Sheet.vue'
import SigninSteps from '../components/SigninSteps.vue'
import {
  cancelSignIn, connect, dismissSignin, getSettings, isRunning, opLabel, removeAccount, sessionAge, signIn, signingIn, signOut, store,
  type AccountView,
} from '../lib/accounts'
import { unlockName } from '../lib/server'

const editing = ref<AccountView | null>(null)
const adding = ref(false)
const removing = ref<AccountView | null>(null)
const removeError = ref<string | null>(null)
const watching = ref<string | null>(null)
const idleMinutes = ref(30)
const now = ref(Date.now())
let tick: ReturnType<typeof setInterval> | undefined

onMounted(async () => {
  // Started before the awaits, so leaving the page early still finds it to clear.
  tick = setInterval(() => (now.value = Date.now()), 30_000)
  await connect()
  idleMinutes.value = (await getSettings()).idleMinutes
})
onBeforeUnmount(() => clearInterval(tick))

const idleText = computed(() => (idleMinutes.value === 60 ? '1 uur' : idleMinutes.value === 240 ? '4 uur' : `${idleMinutes.value} minuten`))

const watched = computed(() => store.accounts.find((a) => a.id === watching.value) ?? null)
const watchedEvent = computed(() => (watching.value ? store.signins[watching.value] : undefined))
const ended = computed(() => !!watchedEvent.value && !isRunning(watchedEvent.value.step))

/** A sign-in this window did not start (from the menu-bar panel) and does not show yet: the row offers to follow it. */
function followable(account: AccountView): boolean {
  return signingIn(account) && watching.value !== account.id
}

/** Shows the steps and the code field of a running sign-in, without starting a new one. */
function follow(account: AccountView) {
  watching.value = account.id
}

function startSignIn(account: AccountView) {
  watching.value = account.id
  void signIn(account.id)
}

function onSaved(account: AccountView, added: boolean) {
  adding.value = false
  editing.value = null
  // A new account signs in once to check its credentials: show those steps.
  if (added) watching.value = account.id
}

function closeSignin() {
  if (!watching.value) return
  if (watched.value && signingIn(watched.value)) void cancelSignIn(watching.value)
  dismissSignin(watching.value)
  watching.value = null
}

async function confirmRemove() {
  if (!removing.value) return
  try {
    await removeAccount(removing.value.id)
    removing.value = null
    removeError.value = null
  } catch (e) {
    removeError.value = String(e)
  }
}

function sessionText(account: AccountView): { tone: string; text: string } {
  if (signingIn(account)) return { tone: 'run', text: 'Bezig met inloggen…' }
  if (account.session?.expired) return { tone: 'bad', text: 'Sessie verlopen' }
  if (account.session) return { tone: 'on', text: sessionAge(account.session.startedAt, now.value) }
  return { tone: 'off', text: account.administration ? 'Niet ingelogd' : 'Nog nooit ingelogd' }
}

</script>

<template>
  <Teleport to="#toolbar-actions" defer>
    <button v-if="store.accounts.length" type="button" class="btn primary" @click="adding = true"><Icon name="plus" :size="15" />Account toevoegen</button>
  </Teleport>

  <EmptyState v-if="store.loaded && !store.accounts.length" icon="users" title="Nog geen accounts">
    Voeg een account toe met je gegevens uit 1Password of de sleutelhanger. EB MCP logt daarmee zelf in bij e-Boekhouden, op de achtergrond.
    <template #actions>
      <button type="button" class="btn primary" @click="adding = true"><Icon name="plus" :size="15" />Account toevoegen</button>
    </template>
  </EmptyState>

  <div v-else class="page">
    <div class="group table" role="table" aria-label="Accounts">
      <div class="row head" role="row">
        <span>Account</span><span>Administratie</span><span>Wachtwoord</span><span>Sessie</span><span />
      </div>
      <div v-for="account in store.accounts" :key="account.id" class="row" role="row">
        <span class="who">
          <Avatar :label="account.label" :muted="!account.session" />
          <span class="stack">
            <span class="name"><span class="name-text" :title="account.label">{{ account.label }}</span></span>
            <span class="sub" :title="account.store === 'onePassword' ? account.username || opLabel(account) : account.username">{{ account.store === 'onePassword' ? account.username || opLabel(account) : account.username }}</span>
          </span>
        </span>
        <span class="cut" :class="{ muted: !account.administration }" :title="account.administration ?? undefined">{{ account.administration ?? 'Na eerste keer inloggen' }}</span>
        <span class="stack">
          <span class="chip" :class="account.store === 'onePassword' ? 'info' : ''">{{ account.store === 'onePassword' ? '1Password' : 'Sleutelhanger' }}</span>
          <span class="sub" :class="{ mono: account.store === 'onePassword' && !account.opTitle }" :title="account.store === 'onePassword' ? opLabel(account) : `${unlockName} bij elk gebruik`">{{ account.store === 'onePassword' ? opLabel(account) : `${unlockName} bij elk gebruik` }}</span>
        </span>
        <span class="session" :class="sessionText(account).tone">
          <span class="dot" :class="{ on: sessionText(account).tone === 'on', off: sessionText(account).tone === 'off', bad: sessionText(account).tone === 'bad' }" />
          <span class="session-text" :title="sessionText(account).text">{{ sessionText(account).text }}</span>
        </span>
        <span class="actions">
          <Transition name="swap-btn" mode="out-in">
            <button v-if="account.session && !account.session.expired" key="out" type="button" class="btn small" @click="signOut(account.id)">Uitloggen</button>
            <button v-else-if="followable(account)" key="follow" type="button" class="btn small" :aria-label="`Inloggen van ${account.label} volgen`" @click="follow(account)">Volgen</button>
            <button v-else key="in" type="button" class="btn small" :class="{ primary: account.session?.expired }" :disabled="account.signingIn" :aria-busy="account.signingIn" @click="startSignIn(account)">
              <span class="btn-label">{{ account.session?.expired ? 'Opnieuw inloggen' : 'Inloggen' }}</span>
            </button>
          </Transition>
          <button type="button" class="icon-btn hov" title="Bewerken" :aria-label="`${account.label} bewerken`" @click="editing = account"><Icon name="pencil" :size="15" /></button>
          <button type="button" class="icon-btn hov danger" title="Verwijderen" :aria-label="`${account.label} verwijderen`" @click="removing = account"><Icon name="trash" :size="15" /></button>
        </span>
      </div>
    </div>

    <div class="cards">
      <section class="group card">
        <h2>Wat EB MCP bewaart</h2>
        <p>Per account alleen de naam, de gebruikersnaam of de 1Password-verwijzing, de administratienaam en waar het wachtwoord staat. Nooit een wachtwoord in de bestanden van de app. Sessies staan alleen in het geheugen en verdwijnen bij uitloggen, afsluiten of stilte.</p>
      </section>
      <section class="group card">
        <h2>Sessies</h2>
        <p>EB MCP vergeet de sessie van een account na {{ idleText }} zonder aanroep van een AI-client; bij e-Boekhouden verloopt die daarna vanzelf. Afsluiten vergeet altijd alle sessies; EB MCP bewaart ze nergens.</p>
        <RouterLink class="btn small" to="/instellingen">Wijzigen in Instellingen</RouterLink>
      </section>
    </div>
  </div>

  <Transition name="modal"><AccountForm v-if="adding" @close="adding = false" @saved="onSaved" /></Transition>
  <Transition name="modal"><AccountForm v-if="editing" :account="editing" @close="editing = null" @saved="onSaved" /></Transition>

  <Transition name="modal">
    <Sheet v-if="watched" :title="`Inloggen bij ${watched.label}`" :width="440" @close="closeSignin">
      <span class="who">
        <Avatar :label="watched.label" :size="32" />
        <span class="stack"><span class="name">{{ watched.label }}</span><span class="sub">{{ watched.store === 'onePassword' ? '1Password' : 'Sleutelhanger' }}</span></span>
      </span>
      <SigninSteps :account="watched" :event="watchedEvent" />
      <template #footer>
        <button v-if="!ended" type="button" class="btn" @click="closeSignin">Annuleren</button>
        <template v-else>
          <button v-if="watchedEvent?.step === 'failed'" type="button" class="btn" @click="editing = watched; closeSignin()">Gegevens wijzigen</button>
          <button v-if="watchedEvent?.step !== 'done'" type="button" class="btn primary" @click="startSignIn(watched)">Opnieuw</button>
          <button type="button" class="btn" :class="{ primary: watchedEvent?.step === 'done' }" @click="closeSignin">Sluiten</button>
        </template>
      </template>
    </Sheet>
  </Transition>

  <Transition name="modal">
    <Alert v-if="removing" :title="`${removing.label} verwijderen?`" tone="danger" @close="removing = null">
      <p>
        EB MCP vergeet de sessie van dit account{{ removing.store === 'system' ? ' en haalt het wachtwoord uit je sleutelhanger' : '' }}.
        {{ removing.store === 'onePassword' ? 'Je 1Password-item blijft staan.' : '' }} Je account bij e-Boekhouden zelf blijft bestaan.
      </p>
      <p v-if="removeError" class="error" role="alert">{{ removeError }}</p>
      <template #buttons>
        <button type="button" class="btn" @click="removing = null">Annuleren</button>
        <button type="button" class="btn danger-fill" @click="confirmRemove">Verwijderen</button>
      </template>
    </Alert>
  </Transition>
</template>

<style scoped>
.page { padding: 8px 28px 28px; display: flex; flex-direction: column; gap: 16px; }
/* At the smallest window (980 px) the table is 700 px wide: 668 inside the row padding, 604 after four gaps. The
   action column keeps its content width (Opnieuw inloggen plus three icon buttons, about 210 px); the four text
   columns share the rest and may shrink to nothing, cutting their text with an ellipsis and a tooltip. */
.table { overflow: hidden; }
.row {
  display: grid; grid-template-columns: minmax(0, 1.6fr) minmax(0, 0.9fr) minmax(0, 0.9fr) minmax(0, 0.9fr) auto; align-items: center; column-gap: 16px;
  min-height: 48px; padding: 0 16px; border-top: 0.5px solid var(--line-soft); transition: background-color 150ms ease;
}
.row:not(.head):hover { background: var(--hover); }
.row.head { min-height: 26px; border-top: 0; background: var(--sunken); font-size: 11px; font-weight: 600; color: var(--label); }
.row.head > span, .cut { min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.who { display: flex; align-items: center; gap: 10px; min-width: 0; }
.stack { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
.name { display: flex; align-items: center; gap: 6px; min-width: 0; font-weight: 600; }
/* A long account name stays on one line; the full name is in the tooltip. */
.name-text { min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.sub { font-size: 11px; color: var(--muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.muted { color: var(--muted); }
.chip { align-self: flex-start; height: 18px; display: inline-flex; align-items: center; padding: 0 7px; border-radius: 999px; background: var(--subtle); color: var(--label); font-size: 11px; font-weight: 500; }
.chip.info { background: var(--info-soft); color: var(--info); }
.session { display: flex; align-items: center; gap: 8px; min-width: 0; font-size: 12px; color: var(--muted); }
.session .dot { flex: none; }
/* A narrow column wraps the status to a second line rather than cutting it. */
.session-text { min-width: 0; line-height: 15px; overflow-wrap: anywhere; display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; line-clamp: 2; overflow: hidden; }
.session.on { color: var(--success); }
.session.bad { color: var(--danger); }
.actions { display: flex; align-items: center; justify-content: flex-end; gap: 2px; }
.actions > * { flex: none; }
.swap-btn-enter-active, .swap-btn-leave-active { transition: opacity 120ms ease; }
.swap-btn-enter-from, .swap-btn-leave-to { opacity: 0; }
.icon-btn { display: inline-flex; align-items: center; justify-content: center; width: 28px; height: 28px; border: 0; border-radius: 8px; background: transparent; color: var(--muted); transition: background-color 150ms ease, color 150ms ease, opacity 150ms ease, transform 120ms var(--ease); }
.icon-btn:hover { background: var(--hover); color: var(--text); }
.icon-btn:active { transform: scale(0.92); }
.icon-btn.danger { color: var(--danger); }
.icon-btn.hov { opacity: 0; }
.row:hover .icon-btn.hov, .row:focus-within .icon-btn.hov { opacity: 1; }
.cards { display: grid; grid-template-columns: 1.3fr 1fr; gap: 16px; }
.card { padding: 16px 18px; display: flex; flex-direction: column; gap: 8px; }
.card h2 { font-size: 13px; font-weight: 700; }
.card p { font-size: 12px; line-height: 17px; color: var(--muted); }
.idle { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
.select { height: 24px; padding: 0 8px; border: 0; border-radius: 7px; background: var(--glass-bar); box-shadow: var(--glass-bar-edge); }
.error { color: var(--danger); }
@media (prefers-reduced-motion: reduce) { .icon-btn:active { transform: none; } }
</style>
