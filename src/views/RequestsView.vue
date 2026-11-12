<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import Alert from '../components/Alert.vue'
import EmptyState from '../components/EmptyState.vue'
import ExportNotice from '../components/ExportNotice.vue'
import Icon from '../components/Icon.vue'
import JsonView from '../components/JsonView.vue'
import {
  clearLog, connectLog, duration, entryDetail, exportLog, isError, log, PHASE_LABELS, phases, proportions, segments, setPaused, size,
  statusLabel, time, timeline, waited, waitingFor, waitNote, waitSteps, workMs, workOffset,
  type EbEntry, type Entry, type Phase, type Segment, type WaitAnswer,
} from '../lib/log'
import { approvalDetail } from '../lib/approvals'
import { useExport } from '../lib/exporting'

type Tab = 'all' | 'mcp' | 'eb'

const tab = ref<Tab>('all')
const search = ref('')
const client = ref('')
const account = ref('')
const errorsOnly = ref(false)
const expanded = ref(new Set<number>())
// Only the row the user just opened or closed animates; live entries under an open row arrive without motion.
const toggled = ref<number | null>(null)
const selectedId = ref<number | null>(null)
const detail = ref<Entry | null>(null)
const clearing = ref(false)
// Exporteer: the file it wrote or why it could not, shown by ExportNotice (shared with Akkoord).
const exported = useExport(exportLog)

const router = useRouter()
// The clock of the waits that still count: "wacht al 4 min" in the list and the detail's figures. It only moves the page
// while a listed call waits for approval or runs after one.
const now = ref(Date.now())
const live = computed(() => log.entries.some((e) => e.kind === 'mcp' && e.waitStartedAt !== null && (e.outcome === 'waiting' || e.outcome === 'running')))
let clock: ReturnType<typeof setInterval> | undefined

onMounted(() => {
  void connectLog()
  clock = setInterval(() => {
    if (live.value) now.value = Date.now()
  }, 1000)
})
onBeforeUnmount(() => clearInterval(clock))

const counts = computed(() => ({
  all: log.entries.length,
  mcp: log.entries.filter((e) => e.kind === 'mcp').length,
  eb: log.entries.filter((e) => e.kind === 'eb').length,
}))
const clients = computed(() => [...new Set(log.entries.map((e) => e.client))].sort())
const accounts = computed(() => [...new Set(log.entries.map((e) => e.account).filter((a): a is string => !!a))].sort())
const filtering = computed(() => !!(search.value || client.value || account.value || errorsOnly.value))

function matches(entry: Entry): boolean {
  if (client.value && entry.client !== client.value) return false
  if (account.value && entry.account !== account.value) return false
  if (errorsOnly.value && !isError(entry)) return false
  const term = search.value.trim().toLowerCase()
  if (term) {
    const text = entry.kind === 'mcp' ? `${entry.tool} ${JSON.stringify(entry.args)}` : entry.path
    if (!text.toLowerCase().includes(term)) return false
  }
  return true
}

const rows = computed(() => {
  if (tab.value === 'eb') return log.entries.filter((e) => e.kind === 'eb' && matches(e)).map((entry) => ({ entry, children: [] as EbEntry[] }))
  if (tab.value === 'mcp') return log.entries.filter((e) => e.kind === 'mcp' && matches(e)).map((entry) => ({ entry, children: [] as EbEntry[] }))
  return timeline(log.entries).filter((r) => matches(r.entry) || r.children.some(matches))
})

// The bars share one scale: the longest work time listed, so a long wait for approval does not shrink every other row.
const maxMs = computed(() => Math.max(1000, ...log.entries.map((e) => workMs(e, now.value))))
/** Some listed call waited for approval: the legend of the bar's colours shows, and Duur takes room for its wait. */
const hasWait = computed(() => rows.value.some(({ entry }) => entry.kind === 'mcp' && entry.waitStartedAt !== null))

interface Dur { work: number; note: string; parts: Segment[]; title: string }
/** The Duur cell of a row: the work time, the wait beside it, and the bar's parts. */
function dur(entry: Entry): Dur {
  const work = workMs(entry, now.value)
  const p = entry.kind === 'mcp' ? phases(entry, now.value) : null
  const title = p
    ? `${duration(work)} aan het werk, ${p.waiting ? `wacht al ${waitingFor(p.waitMs)}` : `${waited(p.waitMs)} gewacht`} op akkoord`
    : duration(work)
  return { work, note: waitNote(entry, now.value), parts: segments(entry, maxMs.value, now.value), title }
}
const durs = computed(() => {
  const map = new Map<number, Dur>()
  for (const { entry } of rows.value) map.set(entry.id, dur(entry))
  return map
})
const PHASES: Phase[] = ['prep', 'wait', 'exec']

function toggle(id: number) {
  toggled.value = id
  const next = new Set(expanded.value)
  if (next.has(id)) next.delete(id)
  else next.add(id)
  expanded.value = next
}

async function select(entry: Entry) {
  selectedId.value = entry.id
  detail.value = await entryDetail(entry.id)
}

watch(
  () => log.entries,
  async () => {
    now.value = Date.now()
    if (selectedId.value !== null) detail.value = await entryDetail(selectedId.value)
  },
)

// ---- The wait of the opened call ----

const detailPhases = computed(() => (detail.value?.kind === 'mcp' ? phases(detail.value, now.value) : null))
/** How the proposal of the opened call was answered, from its record on Akkoord (approval_get), once it was answered. */
const answer = ref<WaitAnswer | null>(null)
let answerFor: number | null = null
let answerSeq = 0
watch(
  () => {
    const d = detail.value
    return d?.kind === 'mcp' && d.waitStartedAt !== null && d.proposal !== undefined ? { id: d.id, proposal: d.proposal, ended: d.waitEndedAt } : null
  },
  async (key, before) => {
    if (key?.id !== answerFor) answer.value = null
    answerFor = key?.id ?? null
    // Read once per opened call when its wait has ended, and again only when that end changes.
    if (!key || key.ended === null || (before && before.id === key.id && before.ended === key.ended)) return
    const seq = ++answerSeq
    try {
      const record = await approvalDetail(key.proposal)
      if (seq !== answerSeq || answerFor !== key.id) return
      const decision = record.decision === 'approved' || record.decision === 'refused' || record.decision === 'lapsed' ? record.decision : null
      answer.value = decision ? { decision, bulk: !!record.bulk } : null
    } catch {
      // The record left the history (it keeps the last 2000): the log's own outcome names the answer.
      if (seq === answerSeq) answer.value = null
    }
  },
  { immediate: true },
)
const steps = computed(() => (detail.value?.kind === 'mcp' ? waitSteps(detail.value, detailCalls.value, answer.value, now.value) : []))
/** A length that can run to minutes: "640 ms", "4,20 s", "4 min 12 s". */
function long(ms: number): string {
  return ms < 60_000 ? duration(ms) : waited(ms)
}

function openProposal(id: number) {
  void router.push({ path: '/akkoord', query: { open: String(id) } })
}

const detailCalls = computed<EbEntry[]>(() => {
  const d = detail.value
  if (!d || d.kind !== 'mcp') return []
  return log.entries.filter((e): e is EbEntry => e.kind === 'eb' && e.parent === d.id).sort((a, b) => a.at - b.at)
})
// The waterfall of the opened call on its work time: a wait for approval is taken out, so the calls keep their size.
const span = computed(() => {
  const calls = detailCalls.value
  const d = detail.value
  if (!d || d.kind !== 'mcp' || !calls.length) return { total: 1 }
  const end = Math.max(workMs(d, now.value), ...calls.map((c) => workOffset(d, c.at) + c.durationMs))
  return { total: Math.max(1, end) }
})
const parentOf = computed(() => {
  const d = detail.value
  if (!d || d.kind !== 'eb' || d.parent === null) return null
  const parent = log.entries.find((e) => e.id === d.parent)
  if (!parent || parent.kind !== 'mcp') return null
  const index = parent.calls.indexOf(d.id) + 1
  return { parent, index, total: parent.calls.length }
})

function clearFilters() {
  search.value = ''
  client.value = ''
  account.value = ''
  errorsOnly.value = false
}

async function confirmClear() {
  await clearLog()
  clearing.value = false
  selectedId.value = null
  detail.value = null
}

function calls(n: number): string {
  return n === 1 ? '1 aanroep' : `${n} aanroepen`
}

function settled(id: number) {
  if (toggled.value === id) toggled.value = null
}

function initials(text: string | null): string {
  if (!text) return ''
  const words = text.split(/\s+/)
  return ((words[0]?.[0] ?? '') + (words[1]?.[0] ?? words[0]?.[1] ?? '')).toUpperCase()
}
</script>

<template>
  <Teleport to="#toolbar-actions" defer>
    <button type="button" class="btn small export" :disabled="exported.exporting" :aria-busy="exported.exporting" @click="exported.run()"><span class="btn-label">Exporteer</span></button>
    <button type="button" class="btn small danger-text" :disabled="!log.entries.length" @click="clearing = true"><Icon name="trash" :size="14" />Leegmaken</button>
  </Teleport>

  <div class="page">
    <ExportNotice :state="exported">Het bestand bevat je verzoeken met de antwoorden van e-Boekhouden, dus ook boekhoudgegevens: deel het alleen met wie dat mag zien.</ExportNotice>
    <div class="filters">
      <div class="tabs" role="tablist" aria-label="Bron">
        <button v-for="t in (['all', 'mcp', 'eb'] as Tab[])" :key="t" type="button" role="tab" :aria-selected="tab === t" class="tab" :class="{ on: tab === t }" @click="tab = t">
          {{ { all: 'Alles', mcp: 'MCP', eb: 'e-Boekhouden' }[t] }}<span class="count">{{ counts[t] }}</span>
        </button>
      </div>
      <input v-model="search" class="search" type="search" placeholder="Zoek op tool of pad" aria-label="Zoek op tool of pad" />
      <select v-model="client" class="pick" :class="{ on: client }" aria-label="Client">
        <option value="">Alle clients</option>
        <option v-for="c in clients" :key="c" :value="c">{{ c }}</option>
      </select>
      <select v-model="account" class="pick" :class="{ on: account }" aria-label="Account">
        <option value="">Alle accounts</option>
        <option v-for="a in accounts" :key="a" :value="a">{{ a }}</option>
      </select>
      <button type="button" class="pick" :class="{ on: errorsOnly }" :aria-pressed="errorsOnly" @click="errorsOnly = !errorsOnly">Alleen fouten</button>
      <button type="button" class="btn small live" :aria-pressed="!log.paused" @click="setPaused(!log.paused)">
        <template v-if="log.paused">Gepauzeerd{{ log.pending ? ` · ${log.pending} nieuw` : '' }}</template>
        <template v-else><span class="dot on pulse" />Live</template>
      </button>
    </div>

    <Transition name="accordion">
      <div v-if="hasWait" class="accordion">
        <div>
          <ul class="legend" aria-label="Kleuren van de duur">
            <li v-for="phase in PHASES" :key="phase"><span class="swatch seg" :class="phase" />{{ PHASE_LABELS[phase] }}</li>
          </ul>
        </div>
      </div>
    </Transition>

    <EmptyState v-if="log.loaded && !log.entries.length" icon="inbox" title="Nog geen verzoeken">
      Elke MCP-aanroep en de aanroepen naar e-Boekhouden die hij veroorzaakt, verschijnen hier. EB MCP bewaart de laatste 1000, ook na een herstart of update, tot je ze leegmaakt.
    </EmptyState>
    <EmptyState v-else-if="log.loaded && !rows.length" icon="list" title="Geen verzoeken voor deze filters">
      Pas de zoekterm of de filters aan.
      <template #actions><button type="button" class="btn" @click="clearFilters">Filters wissen</button></template>
    </EmptyState>

    <div v-else class="split">
      <div class="list" :class="{ waits: hasWait }" role="table" aria-label="Verzoeken">
        <div class="row head" role="row">
          <span class="c-time" role="columnheader">Tijd</span><span class="c-src" role="columnheader">Bron</span><span class="main" role="columnheader">Tool of pad</span><span class="c-status" role="columnheader">Status</span>
          <span class="c-dur right" role="columnheader">Duur</span><span class="c-size right" role="columnheader">Grootte</span><span class="c-who" role="columnheader">Wie</span>
        </div>
        <div class="body" role="rowgroup">
          <template v-for="{ entry, children } in rows" :key="entry.id">
            <div class="row item" role="row" :class="{ sel: selectedId === entry.id }" @click="select(entry)">
              <span class="c-time mono muted small" role="cell">{{ time(entry.at) }}</span>
              <span class="c-src" role="cell"><span class="badge" :class="entry.kind">{{ entry.kind === 'mcp' ? 'MCP' : (entry as EbEntry).method }}</span></span>
              <span class="main" role="cell">
                <template v-if="entry.kind === 'mcp'">
                  <button v-if="tab === 'all' && children.length" type="button" class="chev" :aria-expanded="expanded.has(entry.id)" aria-label="Aanroepen naar e-Boekhouden tonen" @click.stop="toggle(entry.id)">
                    <span :class="{ open: expanded.has(entry.id) }">›</span>
                  </button>
                  <span v-else class="chev-space" />
                  <strong class="mono name" :title="entry.tool">{{ entry.tool }}</strong>
                  <span class="mono muted args">{{ JSON.stringify(entry.args) }}</span>
                  <span v-if="tab === 'all'" class="chip calls" :title="calls(entry.calls.length)">{{ entry.calls.length }}<span class="unit">&nbsp;{{ entry.calls.length === 1 ? 'aanroep' : 'aanroepen' }}</span></span>
                </template>
                <span v-else class="mono path" :title="entry.path">{{ entry.path }}</span>
              </span>
              <span class="c-status status" role="cell" :class="{ bad: isError(entry), warn: entry.kind === 'mcp' && entry.outcome === 'refused' }" :title="statusLabel(entry)"><span class="dot" :class="isError(entry) ? 'bad' : 'on-static'" /><span class="clip">{{ statusLabel(entry) }}</span></span>
              <span class="c-dur right dur" role="cell" :title="durs.get(entry.id)!.title">
                <span class="clip">{{ duration(durs.get(entry.id)!.work) }}</span><span v-if="durs.get(entry.id)!.note" class="waited">{{ durs.get(entry.id)!.note }}</span>
                <span v-if="durs.get(entry.id)!.parts.length" class="bar phased"><span v-for="part in durs.get(entry.id)!.parts" :key="part.phase" class="seg" :class="part.phase" :style="{ width: `${part.width}%` }" /></span>
                <span v-else class="bar"><span :style="{ width: `${Math.max(4, (durs.get(entry.id)!.work / maxMs) * 100)}%` }" :class="{ slow: durs.get(entry.id)!.work >= 1000 }" /></span>
              </span>
              <span class="c-size right muted clip" role="cell">{{ size(entry.bytes) }}</span>
              <span class="c-who who" role="cell" :title="`${entry.client}${entry.account ? ` · ${entry.account}` : ''}`"><span class="tag">{{ initials(entry.client) }}</span><span v-if="entry.account" class="tag acc">{{ initials(entry.account) }}</span></span>
            </div>
            <Transition name="kids" :css="toggled === entry.id" @after-enter="settled(entry.id)" @after-leave="settled(entry.id)">
              <div v-if="tab === 'all' && expanded.has(entry.id)" class="kids" role="rowgroup">
                <div v-for="(child, i) in children" :key="child.id" class="row item child" role="row" :class="{ sel: selectedId === child.id }" @click="select(child)">
                  <span class="c-time mono muted small" role="cell">{{ time(child.at) }}</span>
                  <span class="c-src" role="cell"><span class="badge eb">{{ child.method }}</span></span>
                  <span class="main" role="cell"><span class="tree" :class="{ last: i === children.length - 1 }" /><span class="mono path" :title="child.path">{{ child.path }}</span></span>
                  <span class="c-status status" role="cell" :class="{ bad: isError(child) }" :title="statusLabel(child)"><span class="dot" :class="isError(child) ? 'bad' : 'on-static'" /><span class="clip">{{ statusLabel(child) }}</span></span>
                  <span class="c-dur right dur" role="cell"><span class="clip">{{ duration(child.durationMs) }}</span><span class="bar"><span :style="{ width: `${Math.max(4, (child.durationMs / maxMs) * 100)}%` }" :class="{ slow: child.durationMs >= 1000 }" /></span></span>
                  <span class="c-size right muted clip" role="cell">{{ size(child.bytes) }}</span>
                  <span class="c-who who" role="cell" :title="`${child.client}${child.account ? ` · ${child.account}` : ''}`"><span class="tag">{{ initials(child.client) }}</span><span v-if="child.account" class="tag acc">{{ initials(child.account) }}</span></span>
                </div>
              </div>
            </Transition>
          </template>
        </div>
      </div>

      <Transition name="detail">
        <aside v-if="detail" class="detail" aria-label="Details">
          <div class="detail-head">
            <div class="title-line">
              <span class="badge" :class="detail.kind">{{ detail.kind === 'mcp' ? 'MCP' : detail.method }}</span>
              <strong class="mono title">{{ detail.kind === 'mcp' ? detail.tool : detail.path }}</strong>
              <button type="button" class="icon-btn" aria-label="Details sluiten" title="Sluiten" @click="detail = null; selectedId = null">✕</button>
            </div>
            <div class="facts">
              <span class="chip" :class="isError(detail) ? 'bad' : 'ok'">{{ statusLabel(detail) }}</span>
              <span class="chip" :title="detail.kind === 'mcp' && detailPhases ? 'Aan het werk, zonder het wachten op akkoord' : undefined">{{ duration(workMs(detail, now)) }}</span>
              <span class="chip">{{ size(detail.bytes) }}</span>
              <span class="chip">{{ detail.client }}</span>
              <span v-if="detail.account" class="chip info">{{ detail.account }}</span>
              <span class="chip mono">{{ time(detail.at) }}</span>
              <span v-if="detail.version" class="chip version" title="De versie van EB MCP die dit verzoek vastlegde">App-versie {{ detail.version }}</span>
            </div>
            <button v-if="parentOf" type="button" class="parent" @click="select(parentOf.parent)">
              Door <strong class="mono">{{ parentOf.parent.tool }}</strong> van {{ parentOf.parent.client }}, aanroep {{ parentOf.index }} van {{ parentOf.total }}
            </button>
          </div>
          <div class="detail-body">
            <p v-if="detail.error" class="note bad" role="alert">{{ detail.error }}</p>
            <template v-if="detail.kind === 'mcp'">
              <section v-if="detailPhases" class="wait-view" aria-label="Verloop">
                <dl class="figures">
                  <div class="figure"><dt>Aan het werk</dt><dd>{{ duration(detailPhases.workMs) }}</dd></div>
                  <div class="figure"><dt>Gewacht</dt><dd>{{ detailPhases.waiting ? waitingFor(detailPhases.waitMs) : waited(detailPhases.waitMs) }}</dd></div>
                  <div class="figure"><dt>Totaal</dt><dd>{{ long(detailPhases.totalMs) }}</dd></div>
                </dl>
                <div class="phase-bar" role="img" :aria-label="`Voorbereiden ${duration(detailPhases.prepMs)}, wachten ${waited(detailPhases.waitMs)}, uitvoeren ${duration(detailPhases.execMs)}`">
                  <span v-for="part in proportions(detailPhases)" :key="part.phase" class="seg" :class="part.phase" :style="{ width: `${part.width}%` }" />
                </div>
                <TransitionGroup name="accordion" tag="ol" class="steps">
                  <li v-for="step in steps" :key="step.key" class="accordion" :data-step="step.key">
                    <div>
                      <div class="step">
                        <span class="step-dot" :class="step.tone" aria-hidden="true" />
                        <div class="step-body">
                          <div class="step-head">
                            <span class="step-title">{{ step.title }}</span>
                            <span v-if="step.chip" class="chip step-chip">{{ step.chip }}</span>
                            <span class="step-value">{{ step.value }}</span>
                          </div>
                          <p v-if="step.note" class="step-note selectable">{{ step.note }}</p>
                          <div v-if="step.calls?.length" class="step-calls">
                            <button v-for="call in step.calls" :key="call.id" type="button" class="step-call" :title="call.path" @click="select(call)">
                              <span class="mono method">{{ call.method }}</span><span class="mono path">{{ call.path }}</span>
                              <span class="call-status" :class="{ bad: isError(call) }">{{ statusLabel(call) }}</span><span class="right">{{ duration(call.durationMs) }}</span>
                            </button>
                          </div>
                        </div>
                      </div>
                    </div>
                  </li>
                </TransitionGroup>
                <button v-if="detail.proposal !== undefined" type="button" class="btn small open-proposal" @click="openProposal(detail.proposal)">Bekijk voorstel op Akkoord</button>
              </section>
              <section><h3 class="section-label">Argumenten</h3><JsonView :value="detail.args" /></section>
              <section v-if="detailCalls.length">
                <h3 class="section-label">Naar e-Boekhouden · {{ calls(detailCalls.length) }}, {{ duration(workMs(detail, now)) }}</h3>
                <button v-for="call in detailCalls" :key="call.id" type="button" class="wf" @click="select(call)">
                  <span class="mono path">{{ call.path }}</span>
                  <span class="wf-track"><span class="wf-bar" :class="{ bad: isError(call) }" :style="{ left: `${(workOffset(detail, call.at) / span.total) * 100}%`, width: `${Math.max(2, (call.durationMs / span.total) * 100)}%` }" /></span>
                  <span class="right small">{{ duration(call.durationMs) }}</span>
                </button>
              </section>
              <section v-if="detail.result !== undefined && detail.result !== null"><h3 class="section-label">Resultaat, zoals de client het kreeg</h3><JsonView :value="detail.result" :max-height="320" /></section>
            </template>
            <template v-else>
              <section v-if="detail.query.length">
                <h3 class="section-label">Query</h3>
                <dl class="kv"><template v-for="[k, v] in detail.query" :key="k"><dt class="mono">{{ k }}</dt><dd class="mono selectable">{{ v || '(leeg)' }}</dd></template></dl>
              </section>
              <section>
                <h3 class="section-label">Headers</h3>
                <dl class="kv"><template v-for="[k, v] in detail.headers" :key="k"><dt class="mono">{{ k }}</dt><dd class="mono">{{ v }}<span v-if="v.startsWith('••')" class="chip">verborgen</span></dd></template></dl>
              </section>
              <section v-if="detail.response !== undefined && detail.response !== null">
                <h3 class="section-label">Antwoord<span v-if="detail.rows !== null" class="chip">{{ detail.rows }} rijen</span><span v-if="detail.truncated" class="chip">ingekort</span></h3>
                <JsonView :value="detail.response" :max-height="360" />
              </section>
            </template>
          </div>
        </aside>
      </Transition>
    </div>
  </div>

  <Alert v-if="clearing" :title="`${log.entries.length} verzoeken leegmaken?`" tone="danger" @close="clearing = false">
    <p>EB MCP bewaart de log, ook na een herstart of update. Leegmaken haalt alle aanroepen weg, ook die van eerdere keren, en kan niet terug. Het dashboard telt door.</p>
    <template #buttons>
      <button type="button" class="btn" @click="clearing = false">Annuleren</button>
      <button type="button" class="btn danger-fill" @click="confirmClear">Leegmaken</button>
    </template>
  </Alert>
</template>

<style scoped>
.page { height: 100%; display: flex; flex-direction: column; }
.filters { flex: none; display: flex; align-items: center; gap: 8px; flex-wrap: wrap; padding: 2px 16px 10px; }
.tabs { display: inline-flex; gap: 2px; padding: 2px; border-radius: 999px; background: var(--subtle); }
.tab { height: 24px; padding: 0 11px; border: 0; border-radius: 999px; background: transparent; font-size: 12px; display: inline-flex; gap: 5px; align-items: center; transition: background-color 180ms var(--ease), box-shadow 180ms var(--ease); }
.tab:hover { background: var(--hover); }
.tab.on { background: var(--seg-on); box-shadow: var(--seg-on-shadow); font-weight: 500; }
.count { font-size: 11px; color: var(--muted); font-variant-numeric: tabular-nums; }
.search { box-sizing: border-box; width: 200px; height: 24px; padding: 0 10px; border: 0; border-radius: 8px; background: var(--field-bg); box-shadow: inset 0 0 0 0.5px var(--field); font-size: 12px; outline: none; -webkit-user-select: text; user-select: text; }
.search:focus { box-shadow: inset 0 0 0 1px var(--accent), 0 0 0 3.5px rgba(10, 132, 255, 0.42); }
.pick { height: 24px; padding: 0 10px; border: 0; border-radius: 999px; background: var(--glass-bar); box-shadow: var(--glass-bar-edge); font-size: 12px; transition: background-color 150ms ease; }
.pick.on { background: var(--info-soft); color: var(--info); box-shadow: inset 0 0 0 0.5px var(--accent); }
.live { margin-left: auto; }
.split { flex: 1; min-height: 0; display: flex; gap: 0; }
.list { flex: 1; min-width: 0; display: flex; flex-direction: column; container: requests / inline-size; }
.body { flex: 1; overflow: auto; padding: 4px 0; }
/*
 * Every track is minmax(0, ...) and every cell has min-width 0: no cell can widen its column or the row, and a
 * column that runs short of room shrinks instead of pushing the next one. Text that does not fit ends in an
 * ellipsis inside its own cell (the full text is in its title). Which columns show depends on the width of the
 * list itself (a container query), not the window's, because the detail pane takes room from the list.
 */
.row {
  display: grid; align-items: center; column-gap: 10px;
  grid-template-columns: minmax(0, 92px) minmax(0, 56px) minmax(0, 1fr) minmax(0, 84px) minmax(0, 96px) minmax(0, 62px) minmax(0, 52px);
}
.row > * { min-width: 0; }
.clip, .row.head > *, .c-time { overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
.row.head { height: 26px; padding: 0 16px; background: var(--sunken); font-size: 11px; font-weight: 600; color: var(--label); }
.row.item { height: 28px; margin: 0 8px; padding: 0 8px; border-radius: 6px; transition: background-color 150ms ease; }
.row.item:hover { background: var(--hover); }
.row.item.sel { background: var(--info-soft); }
.row.child .main { padding-left: 4px; }
.right { text-align: right; justify-content: flex-end; }
.muted { color: var(--muted); }
.small { font-size: 11px; }
.badge {
  display: inline-block; vertical-align: middle; box-sizing: border-box; max-width: 100%; height: 18px; padding: 0 7px; border-radius: 999px;
  font-size: 11px; line-height: 18px; font-weight: 500; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.title-line .badge { flex: none; }
.badge.mcp { background: rgba(175, 82, 222, 0.13); color: #7a3fc7; }
.badge.eb { background: var(--info-soft); color: var(--info); font-family: var(--mono); }
.row.item .main { display: flex; align-items: center; gap: 8px; container: toolcell / inline-size; }
/* The tool name gives way before the chip: it shrinks to an ellipsis, the arguments to nothing, the chip never. */
.name, .args, .path { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-width: 0; }
.name { flex: 0 1 auto; }
.args, .path { flex: 1 1 0; }
.args { font-size: 11px; }
.chip.calls { flex: none; margin-left: 0; }
/* A narrow cell keeps only the count (the full text stays in its title); a very narrow one drops the chip. */
@container toolcell (max-width: 239px) { .calls .unit { display: none; } }
@container toolcell (max-width: 109px) { .calls { display: none; } }
.chev, .chev-space { width: 18px; height: 18px; flex: none; }
.chev { border: 0; border-radius: 5px; background: transparent; color: var(--muted); padding: 0; transition: background-color 150ms ease, color 150ms ease, transform 120ms var(--ease); }
.chev:hover { background: var(--hover); color: var(--text); }
.chev:active { transform: scale(0.92); }
.kids-enter-active { transition: opacity 150ms var(--ease), transform 150ms var(--ease); }
.kids-leave-active { transition: opacity 120ms cubic-bezier(0.4, 0, 1, 1); }
.kids-enter-from { opacity: 0; transform: translateY(-4px); }
.kids-leave-to { opacity: 0; }
.chev span { display: inline-block; transition: transform 180ms var(--ease); }
.chev span.open { transform: rotate(90deg); }
.tree { position: relative; width: 18px; height: 28px; flex: none; }
.tree::before { content: ''; position: absolute; left: 8px; top: 0; bottom: 0; width: 1px; background: var(--line); }
.tree.last::before { bottom: 50%; }
.tree::after { content: ''; position: absolute; left: 8px; top: 50%; width: 9px; height: 1px; background: var(--line); }
.chip { display: inline-flex; align-items: center; height: 18px; padding: 0 7px; border-radius: 999px; background: var(--subtle); color: var(--label); font-size: 11px; font-weight: 500; white-space: nowrap; margin-left: 6px; }
.chip.ok { background: var(--success-soft); color: var(--success); margin-left: 0; }
.chip.bad { background: var(--danger-soft); color: var(--danger); margin-left: 0; }
.chip.info { background: var(--info-soft); color: var(--info); }
.status { display: flex; align-items: center; gap: 6px; font-size: 12px; font-weight: 500; color: var(--success); }
.status.bad { color: var(--danger); }
.status.warn { color: var(--warn); }
.dot.on-static { background: var(--success-solid); }
.dur { display: flex; align-items: center; gap: 6px; font-size: 12px; font-variant-numeric: tabular-nums; }
.status .clip, .dur .clip { min-width: 0; }
.bar { width: 32px; height: 4px; border-radius: 2px; background: var(--line-soft); overflow: hidden; flex: none; }
.bar span { display: block; height: 4px; background: #0a84ff; }
.bar span.slow { background: #ff9f0a; }
/*
 * A call that waited for approval: its bar in the phases' colours, the wait striped. Beside the work time a muted
 * "+ 47,3 s gewacht" that gives way first: it shrinks to an ellipsis before the number does, and only a wide list shows it.
 */
.bar.phased { display: flex; }
.bar .seg { flex: none; height: 4px; transition: width 300ms var(--ease); }
.seg.prep { background: var(--phase-prep); }
.seg.wait { background: repeating-linear-gradient(135deg, var(--phase-wait) 0 3px, var(--phase-wait-stripe) 3px 5px); }
.seg.exec { background: var(--phase-exec); }
.waited { display: none; flex: 0 1000 auto; min-width: 0; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; font-size: 11px; color: var(--muted); }
.legend { display: flex; flex-wrap: wrap; gap: 4px 14px; margin: 0; padding: 0 16px 8px; list-style: none; font-size: 11px; color: var(--muted); }
.legend li { display: inline-flex; align-items: center; gap: 6px; white-space: nowrap; }
.swatch { display: inline-block; width: 14px; height: 6px; border-radius: 2px; }
.who { display: flex; gap: 3px; overflow: hidden; }
.tag { flex: none; height: 18px; min-width: 20px; padding: 0 3px; box-sizing: border-box; border-radius: 5px; background: var(--subtle); color: var(--label); font-size: 9px; font-weight: 700; display: inline-flex; align-items: center; justify-content: center; }
.tag.acc { border-radius: 50%; background: var(--info-soft); color: var(--info); }
.detail {
  width: 400px; flex: none; margin: 0 8px 8px 0; border-radius: 14px; overflow: hidden; display: flex; flex-direction: column;
  background: var(--group); box-shadow: var(--group-edge); animation: slide 240ms var(--ease) both;
}
.detail.detail-leave-active { animation: slide-out 140ms cubic-bezier(0.4, 0, 1, 1) both; pointer-events: none; }
@keyframes slide { from { opacity: 0; transform: translateX(12px); } to { opacity: 1; transform: none; } }
@keyframes slide-out { to { opacity: 0; transform: translateX(12px); } }
@keyframes fade-out { to { opacity: 0; } }
.detail-head { padding: 14px 16px 10px; border-bottom: 0.5px solid var(--line-soft); display: flex; flex-direction: column; gap: 8px; }
.title-line { display: flex; align-items: center; gap: 8px; }
.title { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; font-size: 13px; }
.facts { display: flex; flex-wrap: wrap; gap: 6px; }
.facts .chip { margin-left: 0; }
.parent { text-align: left; border: 0; border-radius: 10px; padding: 7px 10px; background: rgba(175, 82, 222, 0.13); color: #7a3fc7; font-size: 12px; transition: background-color 150ms ease; }
.parent:hover { background: rgba(175, 82, 222, 0.2); }
.detail-body { flex: 1; overflow: auto; padding: 12px 16px 18px; display: flex; flex-direction: column; gap: 16px; }
section { display: flex; flex-direction: column; gap: 6px; }
h3 { display: flex; align-items: center; margin: 0; }
.kv { margin: 0; display: grid; grid-template-columns: 110px 1fr; }
.kv dt, .kv dd { margin: 0; padding: 4px 0; border-top: 0.5px solid var(--line-soft); font-size: 12px; }
.kv dt { color: var(--muted); }
.kv dd { word-break: break-all; }
.wf { display: grid; grid-template-columns: minmax(0, 1fr) 1fr 58px; gap: 8px; align-items: center; height: 28px; padding: 0 6px; border: 0; border-radius: 8px; background: transparent; text-align: left; transition: background-color 120ms ease; }
.wf:hover { background: var(--hover); }
.wf .path { font-size: 11px; }
.wf-track { position: relative; height: 8px; border-radius: 3px; background: var(--sunken); }
.wf-bar { position: absolute; top: 0; bottom: 0; border-radius: 3px; background: #0a84ff; }
.wf-bar.bad { background: #d93025; }
.wait-view { gap: 10px; }
.figures { margin: 0; display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 6px; }
.figure { min-width: 0; padding: 7px 9px; border-radius: 10px; background: var(--sunken); }
.figure dt { font-size: 11px; color: var(--muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.figure dd { margin: 2px 0 0; font-size: 15px; font-weight: 600; font-variant-numeric: tabular-nums; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.phase-bar { display: flex; gap: 1px; height: 10px; border-radius: 5px; overflow: hidden; background: var(--sunken); }
.phase-bar .seg { flex: none; height: 100%; transition: width 300ms var(--ease); }
.phase-bar .seg.wait { background: repeating-linear-gradient(135deg, var(--phase-wait) 0 4px, var(--phase-wait-stripe) 4px 7px); }
.steps { margin: 0; padding: 0; list-style: none; }
.step { position: relative; display: grid; grid-template-columns: 14px minmax(0, 1fr); column-gap: 8px; padding-bottom: 10px; }
.step::before { content: ''; position: absolute; left: 6px; top: 14px; bottom: 0; width: 1px; background: var(--line); }
.steps li:last-child .step { padding-bottom: 0; }
.steps li:last-child .step::before { display: none; }
.step-dot { width: 9px; height: 9px; margin: 4px 0 0 2px; border-radius: 50%; background: var(--off); }
.step-dot.prep { background: var(--phase-prep); }
.step-dot.wait { background: var(--phase-wait); }
.step-dot.exec { background: var(--phase-exec); }
.step-dot.ok { background: var(--success-solid); }
.step-dot.bad { background: var(--danger); }
.step-body { min-width: 0; }
.step-head { display: flex; align-items: center; gap: 6px; min-width: 0; font-size: 12px; line-height: 17px; }
.step-title { min-width: 0; font-weight: 500; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.step-head .step-chip { margin-left: 0; flex: none; }
.step-value { flex: none; margin-left: auto; font-size: 11px; color: var(--muted); font-variant-numeric: tabular-nums; }
.step-note { margin: 2px 0 0; font-size: 11px; line-height: 15px; color: var(--muted); overflow-wrap: anywhere; }
.step-calls { margin-top: 4px; display: flex; flex-direction: column; }
.step-call {
  display: grid; grid-template-columns: auto minmax(0, 1fr) auto 52px; column-gap: 6px; align-items: center; height: 22px; padding: 0 6px;
  border: 0; border-radius: 6px; background: transparent; text-align: left; font-size: 11px; transition: background-color 150ms ease;
}
.step-call:hover { background: var(--hover); }
.step-call .method { color: var(--info); }
.call-status { color: var(--success); white-space: nowrap; }
.call-status.bad { color: var(--danger); }
.open-proposal { align-self: flex-start; }
.note { padding: 9px 11px; border-radius: 10px; font-size: 12px; line-height: 16px; }
.note.bad { background: var(--danger-soft); color: var(--danger); }
.icon-btn { width: 24px; height: 24px; border: 0; border-radius: 7px; background: transparent; color: var(--muted); transition: background-color 150ms ease, color 150ms ease, transform 120ms var(--ease); }
.icon-btn:hover { background: var(--hover); color: var(--text); }
.icon-btn:active { transform: scale(0.92); }
@media (max-width: 1240px) { .detail { width: 340px; } }
@media (max-width: 1080px) { .detail { width: 300px; } }
/* A wide list with a wait listed gives Duur room for the wait beside the work time; narrower, Duur is as it always was. */
@container requests (min-width: 900px) {
  .list.waits .row { grid-template-columns: minmax(0, 92px) minmax(0, 56px) minmax(0, 1fr) minmax(0, 84px) minmax(0, 176px) minmax(0, 62px) minmax(0, 52px); }
  .waited { display: block; }
}
/* The list gives up columns from least to most needed: Grootte, then Wie and the duration bar, then Tijd. */
@container requests (max-width: 759px) {
  .row { grid-template-columns: minmax(0, 84px) minmax(0, 56px) minmax(0, 1fr) minmax(0, 80px) minmax(0, 92px) minmax(0, 52px); }
  .c-size { display: none; }
}
@container requests (max-width: 559px) {
  .row { grid-template-columns: minmax(0, 84px) minmax(0, 56px) minmax(0, 1fr) minmax(0, 80px) minmax(0, 60px); }
  .c-who, .bar, .bar.phased { display: none; }
}
@container requests (max-width: 459px) {
  .row { grid-template-columns: minmax(0, 56px) minmax(0, 1fr) minmax(0, 80px) minmax(0, 60px); }
  .c-time { display: none; }
}
@media (prefers-reduced-motion: reduce) {
  .detail { animation: fade-in 150ms ease both; }
  .detail.detail-leave-active { animation: fade-out 140ms cubic-bezier(0.4, 0, 1, 1) both; }
  .chev span, .bar .seg, .phase-bar .seg { transition: none; }
  .chev:active, .icon-btn:active, .kids-enter-from { transform: none; }
}
</style>
