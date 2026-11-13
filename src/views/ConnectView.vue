<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import Alert from '../components/Alert.vue'
import Icon from '../components/Icon.vue'
import JsonView from '../components/JsonView.vue'
import Sheet from '../components/Sheet.vue'
import ClientIcon from '../components/ClientIcon.vue'
import { CLIENT_NAMES, SERVER_NAMES, usesBridge, clientsStatus, link, otherClientSettings, preview, renewToken, revealToken, unlink, type Client, type ClientStatus, type Preview } from '../lib/connect'
import { masked, serverInfo, type ServerInfo } from '../lib/server'
import { connect as watchAccounts, readOnly, store } from '../lib/accounts'

const info = ref<ServerInfo | null>(null)
const clients = ref<ClientStatus[]>([])
const shownToken = ref<string | null>(null)
const copied = ref<string | null>(null)
const confirming = ref<Preview | null>(null)
const revealInPreview = ref(false)
const busy = ref(false)
const failure = ref<string | null>(null)
const done = ref<Client | null>(null)
const renewing = ref(false)
const unlinking = ref<Client | null>(null)
let hideTimer: ReturnType<typeof setTimeout> | null = null
let doneTimer: ReturnType<typeof setTimeout> | null = null

async function load() {
  info.value = await serverInfo()
  clients.value = await clientsStatus()
}
onMounted(load)

// Whether any account lets AI clients change its books: the page says so next to the other safeguards.
onMounted(() => void watchAccounts())
const writes = computed(() => store.accounts.some((a) => !readOnly(a.rights)))

function flash(what: string) {
  copied.value = what
  setTimeout(() => (copied.value = null), 1600)
}

async function copy(text: string, what: string) {
  await navigator.clipboard.writeText(text)
  flash(what)
}

async function toggleToken() {
  if (shownToken.value) {
    shownToken.value = null
    return
  }
  shownToken.value = await revealToken()
  if (hideTimer) clearTimeout(hideTimer)
  hideTimer = setTimeout(() => (shownToken.value = null), 15_000)
}

async function openConfirm(client: Client) {
  failure.value = null
  revealInPreview.value = false
  try {
    confirming.value = await preview(client)
  } catch (e) {
    failure.value = String(e)
  }
}

const previewToken = ref<string | null>(null)
async function togglePreviewToken() {
  revealInPreview.value = !revealInPreview.value
  if (revealInPreview.value && !previewToken.value) previewToken.value = await revealToken()
}
const previewEntry = computed(() => {
  const entry = confirming.value?.entry
  if (!entry || !revealInPreview.value || !previewToken.value) return entry
  const tail = previewToken.value.slice(-4)
  return JSON.parse(JSON.stringify(entry).split(`••••••••••••${tail}`).join(previewToken.value))
})

async function confirmLink() {
  if (!confirming.value || busy.value) return
  busy.value = true
  const client = confirming.value.client
  try {
    await link(client)
    confirming.value = null
    done.value = client
    if (doneTimer) clearTimeout(doneTimer)
    doneTimer = setTimeout(() => (done.value = null), 6000)
  } catch (e) {
    failure.value = String(e)
    confirming.value = null
  } finally {
    busy.value = false
    await load()
  }
}

async function confirmUnlink() {
  if (!unlinking.value) return
  try {
    await unlink(unlinking.value)
  } catch (e) {
    failure.value = String(e)
  }
  unlinking.value = null
  await load()
}

async function confirmRenew() {
  renewing.value = false
  shownToken.value = null
  previewToken.value = null
  await renewToken()
  await load()
}

function stateText(c: ClientStatus): string {
  return { none: 'Niet gekoppeld', linked: 'Gekoppeld', stale: 'Oud token · opnieuw schrijven', broken: 'Bestand is kapot' }[c.state]
}
</script>

<template>
  <div class="page">
    <section class="group card" aria-labelledby="server-title">
      <div class="head">
        <Icon name="server" :size="18" class="head-icon" />
        <h2 id="server-title">Lokale MCP-server</h2>
        <span v-if="info" class="state" :class="info.running ? 'ok' : 'bad'"><span class="dot" :class="info.running ? 'on' : 'bad'" />{{ info.running ? 'Actief' : 'Uit' }}</span>
      </div>
      <p v-if="info?.error" class="error rise" role="alert">{{ info.error }}</p>
      <dl v-if="info" class="facts">
        <dt>Adres</dt>
        <dd class="mono selectable">{{ info.url }}</dd>
        <dd class="actions"><button type="button" class="btn small ghost" @click="copy(info.url, 'url')"><Icon name="copy" :size="14" /><span class="swap"><span :class="{ off: copied === 'url' }" :aria-hidden="copied === 'url'">Kopiëren</span><span :class="{ off: copied !== 'url' }" :aria-hidden="copied !== 'url'">Gekopieerd</span></span></button></dd>
        <dt>Token</dt>
        <dd class="mono selectable">{{ shownToken ?? masked(info.tokenTail) }}</dd>
        <dd class="actions">
          <button type="button" class="btn small ghost" :aria-pressed="!!shownToken" @click="toggleToken">{{ shownToken ? 'Verbergen' : 'Tonen' }}</button>
          <button type="button" class="btn small ghost" @click="revealToken().then((t) => copy(t, 'token'))"><Icon name="copy" :size="14" /><span class="swap"><span :class="{ off: copied === 'token' }" :aria-hidden="copied === 'token'">Kopiëren</span><span :class="{ off: copied !== 'token' }" :aria-hidden="copied !== 'token'">Gekopieerd</span></span></button>
          <button type="button" class="btn small ghost" @click="renewing = true"><Icon name="refresh" :size="14" />Vernieuwen</button>
        </dd>
      </dl>
      <div class="chips">
        <span class="chip">Alleen 127.0.0.1</span><span class="chip">Websites geweigerd (Origin)</span><span class="chip">{{ writes ? 'Boeken alleen waar jij het aanzet' : 'Alleen lezen' }}</span><span v-if="info" class="chip mcp rise">{{ info.toolCount }} {{ info.toolCount === 1 ? 'tool' : 'tools' }}</span>
      </div>
    </section>

    <div class="clients">
      <section v-for="c in clients" :key="c.client" class="group card client">
        <div class="client-head">
          <ClientIcon :client="c.client" />
          <span class="stack">
            <strong>{{ CLIENT_NAMES[c.client] }}</strong>
            <span class="client-state" :class="c.state"><span class="dot" :class="{ on: c.state === 'linked', off: c.state === 'none', bad: c.state === 'broken' || c.state === 'stale' }" />{{ stateText(c) }}</span>
          </span>
        </div>
        <span class="path mono" :title="c.path">{{ c.path }}</span>
        <p v-if="c.error" class="error">{{ c.error }}</p>
        <div class="buttons">
          <button v-if="c.state === 'none'" type="button" class="btn primary" @click="openConfirm(c.client)"><Icon name="plug" :size="15" />Koppelen</button>
          <button v-if="c.state === 'stale' || c.state === 'linked'" type="button" class="btn" :class="{ primary: c.state === 'stale' }" @click="openConfirm(c.client)"><Icon name="refresh" :size="15" />Opnieuw schrijven</button>
          <button v-if="c.state === 'linked' || c.state === 'stale'" type="button" class="btn" @click="unlinking = c.client">Ontkoppelen</button>
        </div>
      </section>
      <section class="group card client other">
        <div class="client-head">
          <span class="client-icon dashed"><Icon name="plug" :size="16" /></span>
          <span class="stack"><strong>Andere client</strong><span class="client-state">Zelf instellen</span></span>
        </div>
        <p class="muted">Elke client die MCP over HTTP spreekt: gebruik het adres hierboven en stuur het token mee als <span class="mono">Authorization: Bearer</span>.</p>
        <div class="buttons"><button type="button" class="btn" @click="otherClientSettings().then((t) => copy(t, 'other'))"><Icon name="copy" :size="15" /><span class="swap"><span :class="{ off: copied === 'other' }" :aria-hidden="copied === 'other'">Kopieer instellingen</span><span :class="{ off: copied !== 'other' }" :aria-hidden="copied !== 'other'">Gekopieerd</span></span></button></div>
      </section>
    </div>

    <Transition name="banner">
      <div v-if="done" class="banner" role="status">
        <img src="/app-icon.png" alt="" width="34" height="34" />
        <span><strong>Gekoppeld met {{ CLIENT_NAMES[done] }}</strong><br />{{ ({ claude: 'Start Claude opnieuw om e-Boekhouden MCP te zien.', claudeCode: 'Nieuwe Claude Code-sessies zien e-boekhouden meteen.', cursor: 'Cursor pakt de wijziging meestal zelf op; anders even opnieuw starten.', codex: 'Start Codex opnieuw om e-boekhouden te zien.' } as const)[done] }}</span>
      </div>
    </Transition>
  </div>

  <Transition name="modal">
    <Sheet v-if="confirming" :title="`Koppelen met ${CLIENT_NAMES[confirming.client]}`" :width="640" @close="confirming = null">
      <p>EB MCP {{ confirming.replaces ? 'vervangt zijn eigen regel' : 'voegt de server toe' }} in het instellingenbestand van {{ CLIENT_NAMES[confirming.client] }}. {{ confirming.otherServers.length ? 'Je andere servers blijven staan.' : '' }}</p>
      <div class="file"><span class="mono">{{ confirming.path }}</span><span class="chip">{{ confirming.exists ? `${confirming.otherServers.length} andere servers` : 'nieuw bestand' }}</span></div>
      <template v-if="confirming.toml">
        <div class="diff mono">
          <span v-for="name in confirming.otherServers" :key="name" class="ctx">  [mcp_servers.{{ name }}] …</span>
        </div>
        <pre class="toml mono">{{ confirming.toml }}</pre>
      </template>
      <template v-else>
        <div class="diff mono">
          <span v-for="name in confirming.otherServers" :key="name" class="ctx">  "{{ name }}": { … }</span>
          <span class="add">+ "{{ SERVER_NAMES[confirming.client] }}":</span>
        </div>
        <JsonView :value="previewEntry" />
      </template>
      <div v-if="!usesBridge(confirming.client)" class="row-line">
        <button type="button" class="btn small" :aria-pressed="revealInPreview" @click="togglePreviewToken">{{ revealInPreview ? 'Token verbergen' : 'Token tonen' }}</button>
        <span class="muted small">Het token staat straks leesbaar in dit bestand, net als bij elke andere server.</span>
      </div>
      <ul class="bullets">
        <li v-if="confirming.exists">Eerst een reservekopie: {{ confirming.backup }}</li>
        <li v-if="confirming.legacy">Een koppeling onder de oude naam eb-mcp maakt plaats voor {{ SERVER_NAMES[confirming.client] }}.</li>
        <li v-if="confirming.client === 'claude'">Claude Desktop start EB MCP via een kleine brug in de app, omdat Claude alleen lokale stdio-servers start.</li>
        <li v-if="confirming.client === 'codex'">Codex start EB MCP via een kleine brug in de app; je andere Codex-instellingen en opmerkingen blijven staan.</li>
        <li v-if="usesBridge(confirming.client)">Het token staat niet in dit bestand: de brug leest het uit een bestand van EB MCP dat alleen jij kunt lezen.</li>
        <li>{{ ({ claude: 'Start Claude daarna opnieuw; Claude leest het bestand alleen bij de start.', claudeCode: 'Start een nieuwe Claude Code-sessie; lopende sessies zien de server pas na een herstart.', cursor: 'Cursor pakt de wijziging meestal zelf op.', codex: 'Start Codex daarna opnieuw; Codex leest config.toml alleen bij de start.' } as const)[confirming.client] }}</li>
      </ul>
      <template #footer>
        <button type="button" class="btn" :disabled="busy" @click="confirming = null">Annuleren</button>
        <button type="button" class="btn primary" :disabled="busy" :aria-busy="busy" @click="confirmLink"><span class="btn-label"><Icon name="plug" :size="15" />Koppelen</span></button>
      </template>
    </Sheet>
  </Transition>

  <Transition name="modal">
    <Alert v-if="failure" title="Koppelen lukte niet" tone="danger" @close="failure = null">
      <p>{{ failure }}</p>
      <p>Er is niets veranderd. Herstel het bestand of zet de reservekopie terug, en probeer het opnieuw.</p>
      <template #buttons><button type="button" class="btn primary" @click="failure = null">OK</button></template>
    </Alert>
  </Transition>

  <Transition name="modal">
    <Alert v-if="renewing" title="Nieuw token maken?" tone="warn" @close="renewing = false">
      <p>Het huidige token werkt dan meteen niet meer. Gekoppelde clients krijgen een fout tot hun bestand opnieuw geschreven is.</p>
      <template #buttons>
        <button type="button" class="btn" @click="renewing = false">Annuleren</button>
        <button type="button" class="btn primary" @click="confirmRenew">Vernieuwen</button>
      </template>
    </Alert>
  </Transition>

  <Transition name="modal">
    <Alert v-if="unlinking" :title="`Ontkoppelen van ${CLIENT_NAMES[unlinking]}?`" @close="unlinking = null">
      <p>EB MCP haalt alleen zijn eigen regel uit het bestand, na een reservekopie.</p>
      <template #buttons>
        <button type="button" class="btn" @click="unlinking = null">Annuleren</button>
        <button type="button" class="btn primary" @click="confirmUnlink">Ontkoppelen</button>
      </template>
    </Alert>
  </Transition>
</template>

<style scoped>
.page { padding: 8px 28px 28px; display: flex; flex-direction: column; gap: 16px; position: relative; }
.card { padding: 18px 20px; display: flex; flex-direction: column; gap: 12px; }
.head { display: flex; align-items: center; gap: 10px; }
.head h2 { flex: 1; font-size: 13px; font-weight: 700; }
.head-icon { color: var(--label); }
.state { display: inline-flex; align-items: center; gap: 6px; height: 20px; padding: 0 8px; border-radius: 999px; font-size: 11px; font-weight: 500; }
.state.ok { background: var(--success-soft); color: var(--success); }
.state.bad { background: var(--danger-soft); color: var(--danger); }
.error { font-size: 12px; color: var(--danger); }
.facts { display: grid; grid-template-columns: 90px minmax(0, 1fr) auto; gap: 8px 14px; align-items: center; margin: 0; }
.facts dt { color: var(--muted); }
.facts dd { margin: 0; word-break: break-all; }
.actions { display: flex; gap: 2px; }
.chips { display: flex; flex-wrap: wrap; gap: 8px; }
.chip { height: 18px; display: inline-flex; align-items: center; padding: 0 7px; border-radius: 999px; background: var(--subtle); color: var(--label); font-size: 11px; font-weight: 500; white-space: nowrap; }
.chip.mcp { background: rgba(175, 82, 222, 0.13); color: #7a3fc7; }
.clients { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 16px; }
.client.other { grid-column: 1 / -1; }
.toml { margin: 0; padding: 10px 12px; border-radius: 10px; background: var(--sunken); font-size: 12px; line-height: 18px; white-space: pre-wrap; word-break: break-all; -webkit-user-select: text; user-select: text; }
.client-head { display: flex; align-items: center; gap: 12px; }
.client-icon { width: 40px; height: 40px; flex: none; border-radius: 10px; background: var(--subtle); color: var(--label); font-weight: 700; display: flex; align-items: center; justify-content: center; }
.client-icon.dashed { background: transparent; border: 1.5px dashed var(--field); box-sizing: border-box; }
.stack { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
.client-state { display: flex; align-items: center; gap: 6px; font-size: 12px; color: var(--muted); }
.client-state.linked { color: var(--success); }
.client-state.stale { color: var(--warn); }
.client-state.broken { color: var(--danger); }
.path { padding: 7px 10px; border-radius: 9px; background: var(--sunken); font-size: 11px; color: var(--label); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.buttons { display: flex; gap: 8px; flex-wrap: wrap; }
.muted { color: var(--muted); font-size: 12px; line-height: 17px; }
.small { font-size: 11px; }
.banner {
  position: fixed; right: 16px; top: 12px; z-index: 25; display: flex; align-items: center; gap: 10px; width: 340px; box-sizing: border-box; padding: 12px 14px; border-radius: 20px;
  background: var(--glass-pop); -webkit-backdrop-filter: blur(40px) saturate(190%); backdrop-filter: blur(40px) saturate(190%); box-shadow: var(--pop-shadow); animation: banner 300ms var(--ease) both;
}
.banner.banner-leave-active { animation: banner-out 150ms cubic-bezier(0.4, 0, 1, 1) both; }
.banner img { border-radius: 8px; }
@keyframes banner { from { opacity: 0; transform: translateX(24px) scale(0.98); } to { opacity: 1; transform: none; } }
@keyframes banner-out { to { opacity: 0; transform: translateX(24px); } }
.file { display: flex; align-items: center; gap: 8px; padding: 7px 10px; border-radius: 9px; background: var(--sunken); font-size: 12px; }
.file .mono { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.diff { display: flex; flex-direction: column; font-size: 12px; line-height: 18px; }
.diff .ctx { color: var(--muted); }
.diff .add { color: var(--success); }
.row-line { display: flex; align-items: center; gap: 8px; }
.bullets { margin: 0; padding-left: 18px; font-size: 12px; line-height: 18px; color: var(--label); }
@media (prefers-reduced-motion: reduce) {
  .banner { animation: fade-in 150ms ease both; }
  .banner.banner-leave-active { animation: banner-fade-out 150ms cubic-bezier(0.4, 0, 1, 1) both; }
}
@keyframes banner-fade-out { to { opacity: 0; } }
</style>
