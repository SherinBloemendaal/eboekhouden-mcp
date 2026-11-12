<script setup lang="ts">
// Akkoord: the changes AI clients asked for in accounts set to "met akkoord". Wachtrij holds the ones that wait, as
// a list with the chosen one beside it; the client does not wait for them, and EB MCP makes a change in the
// background once it is approved. Geschiedenis holds every proposal with what came of it. Only a Goedkeuren click
// (or a confirmed Alles goedkeuren) approves.
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import { useRoute } from 'vue-router'
import Alert from '../components/Alert.vue'
import EmptyState from '../components/EmptyState.vue'
import ExportNotice from '../components/ExportNotice.vue'
import Icon from '../components/Icon.vue'
import JsonView from '../components/JsonView.vue'
import { store } from '../lib/accounts'
import {
  ago, approvalDetail, approvalState, connectApprovals, decideAll, decideApproval, decisionLabel, exportApprovals, filterHistory,
  followSelection, nameList, proposalKind, refreshApprovals, refreshHistory, splitTitle, stamp, STATUSES, statusOf, watchHistory,
  type ApprovalDetail, type ApprovalRecord, type ApprovalStatus, type DecideAllResult, type Proposal,
} from '../lib/approvals'
import { useExport } from '../lib/exporting'

type View = 'queue' | 'history'

const route = useRoute()
const view = ref<View>('queue')
const now = ref(Date.now())
const exported = useExport(exportApprovals)

let clock: ReturnType<typeof setInterval> | undefined
let stopHistory: (() => void) | null = null
onMounted(() => {
  void connectApprovals()
  stopHistory = watchHistory()
  clock = setInterval(() => (now.value = Date.now()), 30_000)
})
onBeforeUnmount(() => {
  stopHistory?.()
  clearInterval(clock)
})

function onTabKey(event: KeyboardEvent) {
  if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return
  event.preventDefault()
  view.value = view.value === 'queue' ? 'history' : 'queue'
  void nextTick(() => document.getElementById(`tab-${view.value}`)?.focus())
}

// ---- Wachtrij ----

const queue = computed(() => store.approvals)
/** Accounts that are not signed in now: a change approved for one of them fails, so the page says so first. */
const signedOut = computed(() => new Set(store.loaded ? store.accounts.filter((a) => !a.session || a.session.expired).map((a) => a.id) : []))
const busy = reactive(new Map<number, 'approve' | 'refuse'>())
const bulk = ref<'approve' | 'refuse' | null>(null)
const feedback = ref<{ tone: 'ok' | 'warn' | 'bad'; text: string } | null>(null)

/** The proposal shown beside the list. Kept on a proposal that still waits; see followSelection. */
const current = ref<number | null>(null)
const chosen = computed(() => queue.value.find((p) => p.id === current.value) ?? null)
const position = computed(() => queue.value.findIndex((p) => p.id === current.value) + 1)
const detailPane = ref<HTMLElement | null>(null)

// The first proposal is selected when the queue arrives; when the selected one leaves (answered here or elsewhere,
// or lapsed) the one after it takes its place, or the one before it when it was the last. Declared before the
// `?open=` watchers below, so that on the same change theirs is the last word.
watch(
  queue,
  (next, before) => {
    current.value = followSelection((before ?? []).map((p) => p.id), next.map((p) => p.id), current.value)
  },
  { immediate: true },
)

const rowId = (id: number) => `q-row-${id}`

function focusRow(id: number) {
  const row = document.getElementById(rowId(id))
  row?.focus()
  row?.scrollIntoView?.({ block: 'nearest' })
}

function pick(id: number) {
  current.value = id
}

/** Arrow keys, Home and End move the selection in the list, which is one tab stop (roving tabindex). */
function onListKey(event: KeyboardEvent) {
  const list = queue.value
  if (!list.length) return
  const at = Math.max(0, list.findIndex((p) => p.id === current.value))
  let to: number
  switch (event.key) {
    case 'ArrowDown': to = Math.min(at + 1, list.length - 1); break
    case 'ArrowUp': to = Math.max(at - 1, 0); break
    case 'Home': to = 0; break
    case 'End': to = list.length - 1; break
    default: return
  }
  event.preventDefault()
  const id = list[to]!.id
  current.value = id
  void nextTick(() => focusRow(id))
}

function changes(n: number): string {
  return n === 1 ? '1 wijziging' : `${n} wijzigingen`
}

/** "Relatie verwijderen" and "EB MCP proef wachtrij" from "Relatie verwijderen: EB MCP proef wachtrij". */
const parts = (p: Proposal) => splitTitle(p.title)

async function decide(proposal: Proposal, approve: boolean) {
  if (busy.has(proposal.id) || bulk.value) return
  // Focus on the detail's button would be lost with the detail it sits in: it goes to the proposal selected next.
  const focused = document.activeElement
  const fromDetail = !!focused && !!detailPane.value?.contains(focused)
  busy.set(proposal.id, approve ? 'approve' : 'refuse')
  feedback.value = null
  try {
    await decideApproval(proposal.id, approve)
  } catch (error) {
    // Shown above the list: the row itself leaves when the proposal is gone.
    feedback.value = { tone: 'bad', text: `${proposal.title}: ${String(error)}` }
  } finally {
    busy.delete(proposal.id)
  }
  await nextTick()
  if (!fromDetail || current.value === proposal.id) return
  // The last one answered: the queue's tab keeps the focus while the empty state takes the detail's place.
  if (current.value === null) document.getElementById('tab-queue')?.focus()
  else focusRow(current.value)
}

/** What Alles goedkeuren asks about: the proposals on screen when it was clicked, and only those. */
interface Pending { ids: number[]; accounts: string[]; danger: number; signedOut: string[]; items: { id: number; title: string; account: string; danger: boolean }[] }
/** How many of them the confirmation names; the rest as "en nog N". */
const NAMED = 8
const confirming = ref<Pending | null>(null)
const arrived = computed(() => {
  const pending = confirming.value
  return pending ? queue.value.filter((p) => !pending.ids.includes(p.id)).length : 0
})

function askApproveAll() {
  const list = queue.value
  if (!list.length || bulk.value) return
  confirming.value = { ids: list.map((p) => p.id), accounts: list.map((p) => p.account), danger: list.filter((p) => p.danger).length, signedOut: list.filter((p) => signedOut.value.has(p.accountId)).map((p) => p.account), items: list.map((p) => ({ id: p.id, title: p.title, account: p.account, danger: p.danger })) }
}

function report(result: DecideAllResult, approve: boolean): { tone: 'ok' | 'warn'; text: string } {
  const parts: string[] = []
  const done = result.decided.length
  const gone = result.gone.length
  if (done) parts.push(`${changes(done)} ${approve ? 'goedgekeurd: EB MCP voert ze nu uit, de uitkomst staat in Geschiedenis.' : 'geweigerd.'}`)
  if (gone) parts.push(`${gone === 1 ? '1 wijziging was' : `${gone} wijzigingen waren`} al beantwoord of vervallen en ${gone === 1 ? 'is' : 'zijn'} overgeslagen.`)
  return { tone: gone ? 'warn' : 'ok', text: parts.join(' ') }
}

async function approveAll() {
  const pending = confirming.value
  if (!pending || bulk.value) return
  bulk.value = 'approve'
  feedback.value = null
  try {
    feedback.value = report(await decideAll(pending.ids, true), true)
  } catch (error) {
    feedback.value = { tone: 'bad', text: String(error) }
  } finally {
    bulk.value = null
    confirming.value = null
  }
}

async function refuseAll() {
  const ids = queue.value.map((p) => p.id)
  if (!ids.length || bulk.value) return
  bulk.value = 'refuse'
  feedback.value = null
  try {
    feedback.value = report(await decideAll(ids, false), false)
  } catch (error) {
    feedback.value = { tone: 'bad', text: String(error) }
  } finally {
    bulk.value = null
  }
}

// ---- Geschiedenis ----

const status = ref<ApprovalStatus | ''>('')
const search = ref('')
const records = computed(() => filterHistory(approvalState.history, status.value, search.value))
const selectedId = ref<number | null>(null)
const selected = computed(() => approvalState.history.find((r) => r.id === selectedId.value) ?? null)
const detail = ref<ApprovalDetail | null>(null)
const detailError = ref<string | null>(null)
const detailLoading = ref(false)
const detailView = ref<'human' | 'tech'>('human')
const copied = ref(false)
const copyError = ref<string | null>(null)
let detailSeq = 0
let copiedTimer: ReturnType<typeof setTimeout> | undefined
onBeforeUnmount(() => clearTimeout(copiedTimer))

async function loadDetail(id: number) {
  const seq = ++detailSeq
  detailLoading.value = true
  detailError.value = null
  try {
    const value = await approvalDetail(id)
    if (seq === detailSeq) detail.value = value
  } catch (error) {
    if (seq === detailSeq) detailError.value = String(error)
  } finally {
    if (seq === detailSeq) detailLoading.value = false
  }
}

function select(record: ApprovalRecord) {
  if (selectedId.value === record.id) return
  selectedId.value = record.id
  detail.value = null
  detailView.value = 'human'
  copyError.value = null
  void loadDetail(record.id)
}

function closeDetail() {
  detailSeq += 1
  selectedId.value = null
  detail.value = null
  detailError.value = null
  detailLoading.value = false
}

// A change that ran meanwhile has a result now: the technical data follows the status.
watch(
  () => selected.value?.status,
  (next, before) => {
    if (next && before && next !== before && selectedId.value !== null) void loadDetail(selectedId.value)
  },
)

function clearFilters() {
  status.value = ''
  search.value = ''
}

const technical = computed(() => (detail.value ? JSON.stringify({ args: detail.value.args, body: detail.value.body, result: detail.value.result }, null, 2) : ''))

async function copyJson() {
  copyError.value = null
  try {
    await navigator.clipboard.writeText(technical.value)
    copied.value = true
    clearTimeout(copiedTimer)
    copiedTimer = setTimeout(() => (copied.value = false), 1600)
  } catch (error) {
    copyError.value = `Kopiëren lukte niet: ${String(error)}`
  }
}

// ---- Opening one proposal ----

// `?open=<id>` opens the page on that proposal: a toast's Bekijken, or Verzoeken's Bekijk voorstel op Akkoord. One that
// still waits opens in the queue; one that no longer waits opens in Geschiedenis with its detail, once the queue and
// the history are read and it is clear which of the two holds it.
const opening = ref<number | null>(null)
/** The page went to Geschiedenis for `opening` already; the user may leave it while the record is still looked for. */
let openedHistory = false
watch(
  () => route.query.open,
  (open) => {
    const id = Number(open)
    if (typeof open !== 'string' || !Number.isInteger(id)) return
    opening.value = id
    openedHistory = false
    // Selected right away when it waits, so it shows as it appears and does not wait for the read.
    if (!approvalState.loaded || store.approvals.some((p) => p.id === id)) {
      view.value = 'queue'
      current.value = id
    }
  },
  { immediate: true },
)
watch(
  [opening, () => approvalState.loaded, () => store.approvals, () => approvalState.historyLoaded, () => approvalState.history],
  async () => {
    const id = opening.value
    if (id === null || !approvalState.loaded) return
    if (store.approvals.some((p) => p.id === id)) {
      opening.value = null
      view.value = 'queue'
      current.value = id
      await nextTick()
      document.getElementById(rowId(id))?.scrollIntoView?.({ block: 'nearest' })
      return
    }
    if (!approvalState.historyLoaded) return
    const record = approvalState.history.find((r) => r.id === id)
    if (!openedHistory) {
      openedHistory = true
      view.value = 'history'
    }
    // A history read before this page opened may not hold it yet: the fresh read that follows does.
    if (!record) return
    opening.value = null
    openedHistory = false
    view.value = 'history'
    status.value = ''
    search.value = ''
    select(record)
    await nextTick()
    document.getElementById(`record-${id}`)?.scrollIntoView?.({ block: 'nearest' })
  },
  { immediate: true },
)

function showInQueue(id: number) {
  view.value = 'queue'
  if (queue.value.some((p) => p.id === id)) current.value = id
}

/** The outcome in words, for the detail. */
function outcome(record: ApprovalRecord): { tone: 'ok' | 'bad' | 'info' | 'off'; text: string } {
  switch (record.status) {
    case 'waiting': return { tone: 'info', text: 'Wacht op je akkoord.' }
    case 'running': return { tone: 'info', text: 'EB MCP voert deze wijziging nu uit in e-Boekhouden.' }
    case 'done': return { tone: 'ok', text: 'Uitgevoerd in e-Boekhouden.' }
    case 'failed': return { tone: 'bad', text: `Mislukt: ${record.error ?? 'e-Boekhouden gaf geen reden.'}` }
    // The reason says whether anything went out: a change that was cut off while it ran may be half done.
    case 'refused': return { tone: 'off', text: record.error ?? 'Geweigerd. Er is niets gewijzigd in e-Boekhouden.' }
    case 'lapsed':
      return record.decision === 'lapsed' || record.decision === null
        ? { tone: 'off', text: record.error ?? 'Vervallen zonder antwoord. Er is niets gewijzigd in e-Boekhouden.' }
        : { tone: 'bad', text: record.error ?? 'EB MCP is gestopt terwijl deze wijziging liep. Kijk in e-Boekhouden wat er al is doorgevoerd.' }
  }
}
</script>

<template>
  <Teleport to="#toolbar-actions" defer>
    <button type="button" class="btn small export" :disabled="exported.exporting" :aria-busy="exported.exporting" @click="exported.run()"><span class="btn-label">Exporteer</span></button>
  </Teleport>

  <div class="page">
    <ExportNotice :state="exported">Het bestand bevat elk voorstel met wat EB MCP naar e-Boekhouden stuurde en terugkreeg, dus ook boekhoudgegevens: deel het alleen met wie dat mag zien.</ExportNotice>

    <div class="bar">
      <div class="tabs" role="tablist" aria-label="Weergave" @keydown="onTabKey">
        <button
          id="tab-queue" type="button" role="tab" class="tab" :class="{ on: view === 'queue' }" :aria-selected="view === 'queue'"
          aria-controls="pane-queue" :tabindex="view === 'queue' ? 0 : -1" @click="view = 'queue'"
        >Wachtrij<span class="count">{{ queue.length }}</span></button>
        <button
          id="tab-history" type="button" role="tab" class="tab" :class="{ on: view === 'history' }" :aria-selected="view === 'history'"
          aria-controls="pane-history" :tabindex="view === 'history' ? 0 : -1" @click="view = 'history'"
        >Geschiedenis</button>
      </div>
      <Transition name="fade" mode="out-in">
        <div v-if="view === 'history'" key="filters" class="tools">
          <input v-model="search" class="search" type="search" placeholder="Zoek op titel, account of nummer" aria-label="Zoek op titel, account of nummer" />
          <select v-model="status" class="pick" :class="{ on: status }" aria-label="Status">
            <option value="">Alle statussen</option>
            <option v-for="s in STATUSES" :key="s.value" :value="s.value">{{ s.label }}</option>
          </select>
        </div>
        <div v-else-if="queue.length" key="bulk" class="tools bulk">
          <button type="button" class="btn small refuse-all" :disabled="!!bulk" :aria-busy="bulk === 'refuse'" @click="refuseAll"><span class="btn-label">Alles weigeren</span></button>
          <!-- Plain, not filled: the detail's Goedkeuren is the one filled button on this screen. -->
          <button type="button" class="btn small approve-all" :disabled="!!bulk" :aria-busy="bulk === 'approve'" @click="askApproveAll">
            <span class="btn-label">Alles goedkeuren ({{ queue.length }})</span>
          </button>
        </div>
      </Transition>
    </div>

    <Transition name="accordion">
      <div v-if="feedback && view === 'queue'" class="accordion">
        <div>
          <div class="feedback-slot">
            <div class="note feedback" :class="feedback.tone" :role="feedback.tone === 'bad' ? 'alert' : 'status'">
              <Icon :name="feedback.tone === 'ok' ? 'check' : 'alert'" :size="14" class="note-icon" />
              <p class="feedback-text selectable">{{ feedback.text }}</p>
              <button type="button" class="btn small ghost" @click="feedback = null">Sluiten</button>
            </div>
          </div>
        </div>
      </div>
    </Transition>

    <Transition name="fade" mode="out-in">
      <section v-if="view === 'queue'" id="pane-queue" key="queue" class="pane q-pane"role="tabpanel" aria-labelledby="tab-queue">
        <Transition name="fade" mode="out-in">
          <EmptyState v-if="approvalState.error && !approvalState.loaded" key="error" icon="alert" title="De wachtrij kon niet worden geladen">
            {{ approvalState.error }}
            <template #actions><button type="button" class="btn" @click="refreshApprovals">Opnieuw</button></template>
          </EmptyState>
          <EmptyState v-else-if="approvalState.loaded && !queue.length" key="empty" icon="clipboard-check" title="Niets wacht op je akkoord">
            Vraagt een AI-client een wijziging in een account met akkoord, dan staat die hier. De client wacht er niet op: zodra je hem goedkeurt, voert EB MCP de wijziging uit.
          </EmptyState>
          <div v-else key="list" class="q-split">
            <!-- One tab stop: the selected row takes the focus, the arrow keys move it (roving tabindex). -->
            <TransitionGroup name="accordion" tag="ul" class="q-list" aria-label="Wachtrij" @keydown="onListKey">
              <li v-for="p in queue" :key="p.id" class="accordion">
                <div>
                  <button
                    :id="rowId(p.id)" type="button" class="q-row" :class="{ sel: p.id === current }" :tabindex="p.id === current ? 0 : -1"
                    :aria-current="p.id === current ? 'true' : undefined" aria-controls="q-detail" @click="pick(p.id)"
                  >
                    <span class="kind" :class="proposalKind(p).tone" aria-hidden="true"><Icon :name="proposalKind(p).icon" :size="16" /></span>
                    <span class="q-stack">
                      <span class="q-what">{{ parts(p).what }}</span>
                      <span v-if="parts(p).subject" class="q-subject">{{ parts(p).subject }}</span>
                      <span class="q-meta">{{ p.account }} · <time :datetime="new Date(p.askedAt).toISOString()">{{ ago(p.askedAt, now) }}</time></span>
                      <span v-if="p.danger" class="q-flag bad danger-flag">Niet terug te draaien</span>
                      <span v-if="signedOut.has(p.accountId)" class="q-flag warn signed-out-flag">Niet ingelogd</span>
                    </span>
                    <span class="pill">#{{ p.id }}</span>
                  </button>
                </div>
              </li>
            </TransitionGroup>

            <section id="q-detail" ref="detailPane" class="q-detail" role="region" aria-labelledby="q-detail-title">
              <Transition name="fade" mode="out-in">
                <div v-if="chosen" :key="chosen.id" class="q-detail-in">
                  <header class="q-head">
                    <span class="q-of"><span class="pill">#{{ chosen.id }}</span><span>Voorstel {{ chosen.id }} van {{ chosen.client }}</span></span>
                    <h2 id="q-detail-title" class="q-title selectable">{{ chosen.title }}</h2>
                    <span class="q-asked">{{ chosen.account }} · gevraagd door {{ chosen.client }} · <time :datetime="new Date(chosen.askedAt).toISOString()">{{ ago(chosen.askedAt, now) }}</time></span>
                  </header>
                  <div class="q-body">
                    <p v-if="chosen.danger" class="note bad danger-note" role="note"><Icon name="alert" :size="14" class="note-icon" />Dit is niet terug te draaien vanuit EB MCP.</p>
                    <p v-if="signedOut.has(chosen.accountId)" class="note warn signed-out" role="note"><Icon name="alert" :size="14" class="note-icon" />{{ chosen.account }} is niet ingelogd in EB MCP. Log eerst in bij Accounts, anders mislukt deze wijziging.</p>
                    <div class="q-changes">
                      <h3 class="section-label">Wat er verandert</h3>
                      <dl class="q-lines selectable">
                        <div v-for="(line, i) in chosen.lines" :key="i" class="q-line">
                          <dt>{{ line.label }}</dt>
                          <dd>{{ line.value }}</dd>
                        </div>
                      </dl>
                    </div>
                  </div>
                  <footer class="q-foot">
                    <span class="q-pos">Voorstel #{{ chosen.id }}, {{ position }} van {{ queue.length }} in de wachtrij</span>
                    <span class="q-actions">
                      <button type="button" class="btn refuse" :disabled="busy.has(chosen.id) || !!bulk" :aria-busy="busy.get(chosen.id) === 'refuse'" @click="decide(chosen, false)">
                        <span class="btn-label">Weigeren</span>
                      </button>
                      <button
                        type="button" class="btn approve" :class="chosen.danger ? 'danger-fill' : 'primary'" :disabled="busy.has(chosen.id) || !!bulk"
                        :aria-busy="busy.get(chosen.id) === 'approve'" @click="decide(chosen, true)"
                      ><span class="btn-label">Goedkeuren</span></button>
                    </span>
                  </footer>
                </div>
              </Transition>
            </section>
          </div>
        </Transition>
      </section>

      <section v-else id="pane-history" key="history" class="pane" role="tabpanel" aria-labelledby="tab-history">
        <Transition name="fade" mode="out-in">
          <EmptyState v-if="approvalState.historyError && !approvalState.history.length" key="error" icon="alert" title="De geschiedenis kon niet worden geladen">
            {{ approvalState.historyError }}
            <template #actions><button type="button" class="btn" @click="refreshHistory">Opnieuw</button></template>
          </EmptyState>
          <EmptyState v-else-if="approvalState.historyLoaded && !approvalState.history.length" key="empty" icon="clipboard-check" title="Nog geen geschiedenis">
            Elke wijziging die een AI-client vraagt, met of zonder akkoord, komt hier te staan met wat ervan kwam.
          </EmptyState>
          <EmptyState v-else-if="approvalState.historyLoaded && !records.length" key="none" icon="search" title="Geen wijzigingen voor deze filters">
            Pas de zoekterm of de status aan.
            <template #actions><button type="button" class="btn" @click="clearFilters">Filters wissen</button></template>
          </EmptyState>
          <div v-else key="list" class="split">
            <TransitionGroup name="row" tag="ul" class="history" aria-label="Geschiedenis">
              <li v-for="r in records" :key="r.id">
                <button :id="`record-${r.id}`" type="button" class="record" :class="{ sel: selectedId === r.id }" :aria-current="selectedId === r.id ? 'true' : undefined" @click="select(r)">
                  <span class="r-time">{{ stamp(r.askedAt) }}</span>
                  <span class="r-main">
                    <span class="r-title">{{ r.title }}</span>
                    <span class="r-sub"><span class="nr">Voorstel {{ r.id }}</span> · {{ r.account }} · {{ r.client }}</span>
                  </span>
                  <span class="r-status">
                    <span v-if="r.decision === 'auto'" class="chip auto">Zonder akkoord</span>
                    <span class="chip status" :class="statusOf(r.status).tone">{{ statusOf(r.status).label }}</span>
                  </span>
                </button>
              </li>
            </TransitionGroup>

            <Transition name="detail">
              <aside v-if="selected" class="detail" aria-label="Details">
                <div class="detail-head">
                  <div class="title-line">
                    <strong class="detail-title">{{ selected.title }}</strong>
                    <button type="button" class="icon-btn" aria-label="Details sluiten" title="Sluiten" @click="closeDetail"><Icon name="x" :size="14" /></button>
                  </div>
                  <div class="facts">
                    <span class="chip status" :class="statusOf(selected.status).tone">{{ statusOf(selected.status).label }}</span>
                    <span v-if="selected.decision === 'auto'" class="chip auto">Zonder akkoord</span>
                    <span v-if="selected.danger" class="chip bad">Niet terug te draaien</span>
                  </div>
                  <div class="tabs" role="tablist" aria-label="Soort details">
                    <button type="button" role="tab" class="tab" :class="{ on: detailView === 'human' }" :aria-selected="detailView === 'human'" @click="detailView = 'human'">Overzicht</button>
                    <button type="button" role="tab" class="tab" :class="{ on: detailView === 'tech' }" :aria-selected="detailView === 'tech'" @click="detailView = 'tech'">Technisch</button>
                  </div>
                </div>
                <div class="detail-body">
                  <Transition name="fade" mode="out-in">
                    <div v-if="detailView === 'human'" key="human" class="sections human" role="tabpanel">
                      <div class="note outcome" :class="outcome(selected).tone">
                        <p class="selectable">{{ outcome(selected).text }}</p>
                        <button v-if="selected.status === 'waiting'" type="button" class="btn small" @click="showInQueue(selected.id)">Naar de wachtrij</button>
                      </div>
                      <section>
                        <h3 class="section-label">Wat</h3>
                        <dl class="lines selectable">
                          <template v-for="(line, i) in selected.lines" :key="i">
                            <dt>{{ line.label }}</dt>
                            <dd>{{ line.value }}</dd>
                          </template>
                        </dl>
                      </section>
                      <section>
                        <h3 class="section-label">Wie en wanneer</h3>
                        <dl class="kv">
                          <dt>Voorstel</dt><dd class="nr">{{ selected.id }}</dd>
                          <dt>Gevraagd door</dt><dd>{{ selected.client }}</dd>
                          <dt>Account</dt><dd>{{ selected.account }}</dd>
                          <dt>Gevraagd</dt><dd>{{ stamp(selected.askedAt, true) }}</dd>
                          <dt>Beantwoord</dt><dd>{{ selected.decidedAt === null ? 'Nog niet' : stamp(selected.decidedAt, true) }}</dd>
                          <dt>Antwoord</dt><dd>{{ decisionLabel(selected.decision, selected.bulk) }}</dd>
                          <dt>App-versie</dt><dd class="version">{{ selected.version || 'Onbekend' }}</dd>
                        </dl>
                      </section>
                    </div>
                    <div v-else key="tech" class="sections tech" role="tabpanel">
                      <Transition name="fade" mode="out-in">
                        <p v-if="detailError" key="error" class="note bad selectable" role="alert">{{ detailError }}</p>
                        <p v-else-if="!detail" key="loading" class="loading" aria-busy="true">Technische gegevens laden…</p>
                        <div v-else key="data" class="sections">
                          <div class="tech-head">
                            <span class="mono tool">{{ detail.tool }}</span>
                            <button type="button" class="btn small copy" @click="copyJson">
                              <Icon name="copy" :size="14" />
                              <span class="swap"><span :class="{ off: copied }" :aria-hidden="copied">Kopieer JSON</span><span :class="{ off: !copied }" :aria-hidden="!copied">Gekopieerd</span></span>
                            </button>
                          </div>
                          <Transition name="accordion">
                            <div v-if="copyError" class="accordion"><div><p class="note bad" role="alert">{{ copyError }}</p></div></div>
                          </Transition>
                          <section><h3 class="section-label">Argumenten van de client</h3><JsonView :value="detail.args" :max-height="240" /></section>
                          <section v-if="detail.body !== null && detail.body !== undefined"><h3 class="section-label">Naar e-Boekhouden</h3><JsonView :value="detail.body" :max-height="240" /></section>
                          <section v-if="detail.result !== null && detail.result !== undefined"><h3 class="section-label">Uitkomst</h3><JsonView :value="detail.result" :max-height="320" /></section>
                        </div>
                      </Transition>
                    </div>
                  </Transition>
                </div>
              </aside>
            </Transition>
          </div>
        </Transition>
      </section>
    </Transition>
  </div>

  <Transition name="modal">
    <Alert v-if="confirming" :title="`${changes(confirming.ids.length)} goedkeuren?`" :tone="confirming.danger ? 'danger' : undefined" @close="confirming = null">
      <p>Voor {{ nameList(confirming.accounts) }}. EB MCP voert ze daarna een voor een uit in e-Boekhouden.</p>
      <ul class="confirm-list selectable">
        <li v-for="item in confirming.items.slice(0, NAMED)" :key="item.id" :class="{ danger: item.danger }"><span class="confirm-account">{{ item.id }} · </span>{{ item.title }}<span class="confirm-account"> · {{ item.account }}</span></li>
        <li v-if="confirming.items.length > NAMED" class="more">en nog {{ confirming.items.length - NAMED }}</li>
      </ul>
      <p v-if="confirming.danger" class="alert-danger">
        {{ confirming.danger === confirming.ids.length ? (confirming.danger === 1 ? 'Deze is' : 'Ze zijn allemaal') : `${confirming.danger} daarvan ${confirming.danger === 1 ? 'is' : 'zijn'}` }} niet terug te draaien.
      </p>
      <p v-if="confirming.signedOut.length">{{ nameList(confirming.signedOut) }} {{ new Set(confirming.signedOut).size === 1 ? 'is' : 'zijn' }} niet ingelogd in EB MCP: die wijzigingen mislukken tot je daar weer inlogt.</p>
      <p v-if="arrived">Intussen {{ arrived === 1 ? 'kwam er 1 wijziging' : `kwamen er ${arrived} wijzigingen` }} bij; die {{ arrived === 1 ? 'blijft' : 'blijven' }} wachten.</p>
      <template #buttons>
        <button type="button" class="btn" @click="confirming = null">Annuleren</button>
        <button type="button" class="btn confirm-all" :class="confirming.danger ? 'danger-fill' : 'primary'" :disabled="bulk === 'approve'" :aria-busy="bulk === 'approve'" @click="approveAll">
          <span class="btn-label">Goedkeuren</span>
        </button>
      </template>
    </Alert>
  </Transition>
</template>

<style scoped>
.page { height: 100%; display: flex; flex-direction: column; }
.bar { flex: none; display: flex; align-items: center; gap: 8px; flex-wrap: wrap; padding: 2px 16px 10px; }
.tools { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
.tools.bulk { margin-left: auto; }
.tabs { display: inline-flex; align-self: flex-start; gap: 2px; padding: 2px; border-radius: 999px; background: var(--subtle); }
.tab { height: 24px; padding: 0 11px; border: 0; border-radius: 999px; background: transparent; font-size: 12px; display: inline-flex; gap: 5px; align-items: center; transition: background-color 180ms var(--ease), box-shadow 180ms var(--ease); }
.tab:hover { background: var(--hover); }
.tab.on { background: var(--seg-on); box-shadow: var(--seg-on-shadow); font-weight: 500; }
.tab:focus-visible { outline: none; box-shadow: 0 0 0 3px rgba(10, 132, 255, 0.42); }
.count { font-size: 11px; color: var(--muted); font-variant-numeric: tabular-nums; }
.search { box-sizing: border-box; width: 220px; height: 24px; padding: 0 10px; border: 0; border-radius: 8px; background: var(--field-bg); box-shadow: inset 0 0 0 0.5px var(--field); font-size: 12px; outline: none; -webkit-user-select: text; user-select: text; transition: box-shadow 150ms ease; }
.search:focus { box-shadow: inset 0 0 0 1px var(--accent), 0 0 0 3.5px rgba(10, 132, 255, 0.42); }
.pick { height: 24px; padding: 0 10px; border: 0; border-radius: 999px; background: var(--glass-bar); box-shadow: var(--glass-bar-edge); font-size: 12px; transition: background-color 150ms ease, color 150ms ease; }
.pick.on { background: var(--info-soft); color: var(--info); box-shadow: inset 0 0 0 0.5px var(--accent); }

.note { margin: 0; padding: 9px 11px; border-radius: 10px; font-size: 12px; line-height: 16px; }
.note.ok { background: var(--success-soft); color: var(--success); }
.note.bad { background: var(--danger-soft); color: var(--danger); }
.note.warn { background: var(--warn-soft); color: var(--warn); }
.note.info { background: var(--info-soft); color: var(--info); }
.note.off { background: var(--subtle); color: var(--label); }
.note-icon { flex: none; margin-top: 1px; }
.feedback-slot { padding: 0 16px 10px; }
.note.feedback { display: flex; align-items: flex-start; gap: 8px; padding: 5px 6px 5px 11px; }
.feedback .note-icon { margin-top: 5px; }
.feedback-text { flex: 1; min-width: 0; padding: 3px 0; overflow-wrap: anywhere; }
.feedback .btn { flex: none; color: inherit; }

.pane { flex: 1; min-height: 0; display: flex; flex-direction: column; }
/* Wachtrij: the list on the left, the chosen proposal beside it. The pane is the container, so a narrow content
   area (a small main window) stacks the two instead of squeezing them. */
.q-pane { container: queue / inline-size; }
.q-split { flex: 1; min-height: 0; display: flex; gap: 12px; padding: 0 16px 16px; }
.q-list {
  box-sizing: border-box; width: 372px; flex: none; margin: 0; padding: 6px; list-style: none; overflow: auto;
  border-radius: 14px; background: var(--group); box-shadow: var(--group-edge);
}
/* The gap sits inside each collapsing item, so a row that leaves takes its gap with it. */
.q-row {
  box-sizing: border-box; width: 100%; display: flex; align-items: flex-start; gap: 10px; margin-bottom: 2px; padding: 10px; border: 0;
  border-radius: 10px; background: transparent; text-align: left; transition: background-color 150ms ease, box-shadow 150ms ease;
}
.q-row:hover { background: var(--hover); }
.q-row.sel, .q-row.sel:hover { background: var(--info-soft); }
.q-row:focus-visible { outline: none; box-shadow: 0 0 0 3px rgba(10, 132, 255, 0.42); }
.kind {
  flex: none; width: 30px; height: 30px; border-radius: 9px; display: flex; align-items: center; justify-content: center;
  background: var(--subtle); color: var(--label); transition: background-color 150ms ease, color 150ms ease;
}
.kind.accent { background: var(--info-soft); color: var(--info); }
.kind.danger { background: var(--danger-soft); color: var(--danger); }
.q-stack { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
.q-what { font-weight: 600; overflow-wrap: anywhere; }
.q-subject { color: var(--label); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.q-meta { font-size: 11px; color: var(--muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.q-flag { font-size: 11px; font-weight: 600; }
.q-flag.bad { color: var(--danger); }
.q-flag.warn { color: var(--warn); }
.pill {
  flex: none; box-sizing: border-box; height: 20px; display: inline-flex; align-items: center; padding: 0 7px; border-radius: 6px;
  background: var(--subtle); color: var(--text); font-family: var(--mono); font-size: 12px; font-weight: 600; font-variant-numeric: tabular-nums;
}

.q-detail { flex: 1; min-width: 0; min-height: 0; gap: 0; border-radius: 14px; overflow: hidden; background: var(--group); box-shadow: var(--group-edge); }
.q-detail-in { flex: 1; min-height: 0; display: flex; flex-direction: column; }
/* The old proposal fades out under the new one: it takes no clicks meanwhile. */
.q-detail-in.fade-leave-active { pointer-events: none; }
.q-head { flex: none; padding: 20px 24px 16px; display: flex; flex-direction: column; gap: 8px; }
.q-of { display: flex; align-items: center; gap: 8px; min-width: 0; font-size: 12px; color: var(--muted); }
.q-title { font-size: 20px; line-height: 25px; font-weight: 700; overflow-wrap: anywhere; }
.q-asked { font-size: 12px; color: var(--muted); overflow-wrap: anywhere; }
/* Only the body scrolls: the header says what it is, the footer keeps the buttons in reach. */
.q-body { flex: 1; min-height: 0; overflow: auto; padding: 0 24px 18px; display: flex; flex-direction: column; gap: 16px; }
.q-body .note { display: flex; align-items: flex-start; gap: 8px; line-height: 17px; }
.q-changes { display: flex; flex-direction: column; }
.q-changes .section-label { padding-bottom: 6px; }
.q-lines { margin: 0; border-bottom: 0.5px solid var(--line); }
.q-line { display: grid; grid-template-columns: 150px minmax(0, 1fr); column-gap: 16px; padding: 9px 0; border-top: 0.5px solid var(--line); }
.q-line dt { color: var(--muted); overflow-wrap: anywhere; }
/* pre-line: a message or a list of links keeps the line breaks it was given. */
.q-line dd { margin: 0; overflow-wrap: anywhere; white-space: pre-line; }
.q-foot { flex: none; display: flex; align-items: center; flex-wrap: wrap; gap: 8px; padding: 12px 24px; border-top: 0.5px solid var(--line-soft); }
.q-pos { font-size: 12px; color: var(--muted); }
.q-actions { margin-left: auto; display: flex; gap: 8px; }
.q-actions .btn { min-width: 100px; }

@container queue (max-width: 759px) {
  /* Stacked: the list on top, limited and scrolling; the detail under it. When even that does not fit, the pane
     scrolls as a whole, so nothing is cut off. */
  .q-split { flex-direction: column; overflow: auto; }
  .q-list { width: auto; max-height: 216px; }
  .q-detail { flex: 1 0 300px; }
  .q-head { padding: 16px 16px 12px; }
  .q-body { padding: 0 16px 16px; }
  .q-foot { padding: 10px 16px; }
}
@container queue (max-width: 459px) {
  .q-line { grid-template-columns: minmax(0, 1fr); row-gap: 2px; }
}

.nr { font-variant-numeric: tabular-nums; }
.lines { display: grid; grid-template-columns: minmax(0, auto) minmax(0, 1fr); column-gap: 14px; row-gap: 6px; margin: 0; padding: 12px; border-radius: 10px; background: var(--sunken); }
.lines dt { color: var(--label); font-size: 12px; line-height: 18px; white-space: nowrap; }
/* pre-line: a message or a list of links keeps the line breaks it was given. */
.lines dd { margin: 0; line-height: 18px; overflow-wrap: anywhere; white-space: pre-line; }

.chip { display: inline-flex; align-items: center; gap: 4px; height: 18px; padding: 0 7px; border-radius: 999px; background: var(--subtle); color: var(--label); font-size: 11px; font-weight: 500; white-space: nowrap; flex: none; }
.chip.ok { background: var(--success-soft); color: var(--success); }
.chip.bad { background: var(--danger-soft); color: var(--danger); }
.chip.run { background: var(--info-soft); color: var(--info); }
.chip.off { background: var(--subtle); color: var(--muted); }
.chip.auto { background: transparent; color: var(--muted); box-shadow: inset 0 0 0 0.5px var(--line); }

.split { flex: 1; min-height: 0; display: flex; }
.history { flex: 1; min-width: 0; margin: 0; padding: 0 8px 8px; list-style: none; overflow: auto; container: history / inline-size; }
.record {
  box-sizing: border-box; width: 100%; display: grid; grid-template-columns: minmax(0, 100px) minmax(0, 1fr) auto; align-items: center; column-gap: 12px;
  min-height: 40px; padding: 4px 8px; border: 0; border-radius: 8px; background: transparent; text-align: left; transition: background-color 150ms ease;
}
.record:hover { background: var(--hover); }
.record.sel { background: var(--info-soft); }
.record:focus-visible { outline: none; box-shadow: 0 0 0 3px rgba(10, 132, 255, 0.42); }
.r-time { font-size: 11px; color: var(--muted); font-variant-numeric: tabular-nums; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.r-main { min-width: 0; display: flex; flex-direction: column; }
.r-title { font-weight: 500; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.r-sub { font-size: 11px; color: var(--muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.r-status { display: flex; gap: 4px; justify-content: flex-end; }
@container history (max-width: 459px) {
  .record { grid-template-columns: minmax(0, 1fr) auto; }
  .r-time, .r-status .auto { display: none; }
}
.row-enter-active { transition: opacity 150ms var(--ease), transform 150ms var(--ease); }
.row-leave-active { transition: opacity 120ms cubic-bezier(0.4, 0, 1, 1); }
.row-enter-from { opacity: 0; transform: translateY(-4px); }
.row-leave-to { opacity: 0; }

/* A read-only detail beside the list, as Verzoeken has it. */
.detail {
  width: 400px; flex: none; margin: 0 8px 8px 0; border-radius: 14px; overflow: hidden; display: flex; flex-direction: column;
  background: var(--group); box-shadow: var(--group-edge); animation: slide 240ms var(--ease) both;
}
.detail.detail-leave-active { animation: slide-out 140ms cubic-bezier(0.4, 0, 1, 1) both; pointer-events: none; }
@keyframes slide { from { opacity: 0; transform: translateX(12px); } to { opacity: 1; transform: none; } }
@keyframes slide-out { to { opacity: 0; transform: translateX(12px); } }
@keyframes fade-out { to { opacity: 0; } }
.detail-head { padding: 14px 16px 10px; border-bottom: 0.5px solid var(--line-soft); display: flex; flex-direction: column; gap: 8px; }
.title-line { display: flex; align-items: flex-start; gap: 8px; }
.detail-title { flex: 1; min-width: 0; font-size: 13px; line-height: 18px; overflow-wrap: anywhere; padding-top: 3px; }
.facts { display: flex; flex-wrap: wrap; gap: 6px; }
.detail-body { flex: 1; overflow: auto; padding: 12px 16px 18px; }
.sections { display: flex; flex-direction: column; gap: 16px; }
section { display: flex; flex-direction: column; gap: 6px; }
h3 { margin: 0; }
.note.outcome { display: flex; align-items: center; gap: 8px; }
.note.outcome p { flex: 1; min-width: 0; overflow-wrap: anywhere; }
.note.outcome .btn { flex: none; }
.kv { margin: 0; display: grid; grid-template-columns: 110px minmax(0, 1fr); }
.kv dt, .kv dd { margin: 0; padding: 4px 0; border-top: 0.5px solid var(--line-soft); font-size: 12px; }
.kv dt { color: var(--muted); }
.kv dd { overflow-wrap: anywhere; }
.tech-head { display: flex; align-items: center; gap: 8px; }
.tool { flex: 1; min-width: 0; font-size: 12px; color: var(--label); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.tech-head .btn { flex: none; }
.loading { font-size: 12px; color: var(--muted); }
.icon-btn { flex: none; width: 24px; height: 24px; border: 0; border-radius: 7px; background: transparent; color: var(--muted); display: inline-flex; align-items: center; justify-content: center; transition: background-color 150ms ease, color 150ms ease, transform 120ms var(--ease); }
.icon-btn:hover { background: var(--hover); color: var(--text); }
.icon-btn:active { transform: scale(0.92); }
.alert-danger { font-weight: 600; color: var(--danger); }
.confirm-list { margin: 0; padding: 8px 10px; list-style: none; max-height: 168px; overflow: auto; border-radius: 8px; background: var(--sunken); text-align: left; font-size: 12px; line-height: 18px; }
.confirm-list li { overflow-wrap: anywhere; }
.confirm-list li.danger { color: var(--danger); }
.confirm-list .confirm-account, .confirm-list .more { color: var(--muted); }
/* Below the default window width (1240): the detail keeps its full 400 at the size the app opens in. */
@media (max-width: 1239px) { .detail { width: 340px; } }
@media (max-width: 1080px) { .detail { width: 300px; } }
@media (prefers-reduced-motion: reduce) {
  .detail { animation: fade-in 150ms ease both; }
  .detail.detail-leave-active { animation: fade-out 140ms cubic-bezier(0.4, 0, 1, 1) both; }
  .icon-btn:active, .row-enter-from { transform: none; }
}
</style>
